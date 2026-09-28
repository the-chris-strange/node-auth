//! Bun (`bunfig.toml`) registry discovery and local environment authentication.

use crate::AuthError;
use crate::registry::{RegistryPolicy, RegistryUrl};
use crate::token::validate_token;
use std::str::FromStr;
use toml_edit::{DocumentMut, InlineTable, Item, Value};

const TOKEN_VARIABLE: &str = "NODE_AUTH_TOKEN";
const TOKEN_REFERENCE: &str = "$NODE_AUTH_TOKEN";

/// Find eligible registries configured in a Bun configuration file.
pub fn registry_urls(
    content: &str,
    allow_all_domains: bool,
) -> Result<Vec<RegistryUrl>, AuthError> {
    let document = parse(content)?;
    let Some(install) = document.get("install") else {
        return Ok(Vec::new());
    };
    let install = install.as_table_like().ok_or_else(|| {
        AuthError::Config("Bun `install` configuration must be a table".to_string())
    })?;
    let policy = RegistryPolicy { allow_all_domains };
    let mut registries = Vec::new();

    if let Some(item) = install.get("registry")
        && let Some(registry) = parse_registry_item(item, "install.registry", &policy)?
    {
        registries.push(registry);
    }

    if let Some(scopes) = install.get("scopes") {
        let scopes = scopes.as_table_like().ok_or_else(|| {
            AuthError::Config("Bun `install.scopes` configuration must be a table".to_string())
        })?;
        for (scope, item) in scopes.iter() {
            let location = format!("install.scopes.{scope}");
            if let Some(registry) = parse_registry_item(item, &location, &policy)? {
                registries.push(registry);
            }
        }
    }

    registries.sort_by(|left, right| left.npm_key().cmp(right.npm_key()));
    registries.dedup_by(|left, right| left.npm_key() == right.npm_key());
    Ok(registries)
}

/// Replace eligible Bun authentication entries with a `NODE_AUTH_TOKEN` reference.
///
/// Returns `None` when the configuration has no eligible registry.
pub fn transform_bunfig_for_env(
    content: &str,
    allow_all_domains: bool,
) -> Result<Option<String>, AuthError> {
    let mut document = parse(content)?;
    let eligible = registry_urls(content, allow_all_domains)?;
    if eligible.is_empty() {
        return Ok(None);
    }
    let policy = RegistryPolicy { allow_all_domains };
    let install = document
        .get_mut("install")
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| {
            AuthError::Config("Bun `install` configuration must be a table".to_string())
        })?;

    if let Some(item) = install.get_mut("registry") {
        update_registry_item(item, "install.registry", &policy)?;
    }
    if let Some(scopes) = install.get_mut("scopes") {
        let scopes = scopes.as_table_like_mut().ok_or_else(|| {
            AuthError::Config("Bun `install.scopes` configuration must be a table".to_string())
        })?;
        for (scope, item) in scopes.iter_mut() {
            update_registry_item(item, &format!("install.scopes.{scope}"), &policy)?;
        }
    }
    Ok(Some(document.to_string()))
}

/// Add or refresh `NODE_AUTH_TOKEN` in dotenv content while preserving unrelated lines.
pub fn transform_env_local(content: &str, token: &str) -> Result<String, AuthError> {
    validate_token(token)?;
    let newline = if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut output = String::with_capacity(content.len() + token.len() + 32);
    let mut replaced = false;

    for line in content.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let body = body.strip_suffix('\r').unwrap_or(body);
        let trimmed = body.trim_start();
        let is_token = trimmed.starts_with("NODE_AUTH_TOKEN=")
            || trimmed.starts_with("export NODE_AUTH_TOKEN=");
        if is_token {
            if !replaced {
                output.push_str(TOKEN_VARIABLE);
                output.push('=');
                output.push_str(token);
                if line.ends_with('\n') {
                    output.push_str(newline);
                }
                replaced = true;
            }
        } else {
            output.push_str(line);
        }
    }

    if !replaced {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push_str(newline);
        }
        output.push_str(TOKEN_VARIABLE);
        output.push('=');
        output.push_str(token);
        output.push_str(newline);
    }
    Ok(output)
}

fn parse(content: &str) -> Result<DocumentMut, AuthError> {
    DocumentMut::from_str(content).map_err(AuthError::from)
}

fn parse_registry_item(
    item: &Item,
    location: &str,
    policy: &RegistryPolicy,
) -> Result<Option<RegistryUrl>, AuthError> {
    let raw = registry_url(item, location)?;
    policy.parse(raw)
}

fn registry_url<'a>(item: &'a Item, location: &str) -> Result<&'a str, AuthError> {
    let value = item.as_value().ok_or_else(|| invalid_registry(location))?;
    if let Some(url) = value.as_str() {
        return Ok(url);
    }
    let table = value
        .as_inline_table()
        .ok_or_else(|| invalid_registry(location))?;
    table
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_registry(location))
}

fn update_registry_item(
    item: &mut Item,
    location: &str,
    policy: &RegistryPolicy,
) -> Result<(), AuthError> {
    if parse_registry_item(item, location, policy)?.is_none() {
        return Ok(());
    }
    let value = item
        .as_value_mut()
        .ok_or_else(|| invalid_registry(location))?;
    if value.is_str() {
        let url = value.clone();
        let mut table = InlineTable::new();
        table.insert("url", url);
        table.insert("token", Value::from(TOKEN_REFERENCE));
        *value = Value::InlineTable(table);
        return Ok(());
    }
    let table = value
        .as_inline_table_mut()
        .ok_or_else(|| invalid_registry(location))?;
    table.remove("username");
    table.remove("password");
    table.insert("token", Value::from(TOKEN_REFERENCE));
    Ok(())
}

fn invalid_registry(location: &str) -> AuthError {
    AuthError::Config(format!(
        "Bun `{location}` must be a registry URL string or an inline table with a string `url`"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_default_and_scoped_registries() {
        let input = r#"[install]
registry = { url = "https://us-npm.pkg.dev/project/default", token = "old" }

[install.scopes]
myorg = "https://europe-npm.pkg.dev/project/scoped"
other = { url = "https://registry.example.com/private", token = "leave" }
"#;
        let registries = registry_urls(input, false).unwrap();
        assert_eq!(registries.len(), 2);
        assert_eq!(
            registries[0].npm_key(),
            "//europe-npm.pkg.dev/project/scoped/"
        );
        assert_eq!(registries[1].npm_key(), "//us-npm.pkg.dev/project/default/");
    }

    #[test]
    fn transforms_only_eligible_entries_and_preserves_comments() {
        let input = r#"# Bun settings
[install.scopes]
myorg = { url = 'https://us-npm.pkg.dev/project/repo', username = "old", password = "$OLD", extra = true } # keep
other = "https://registry.example.com/private"
"#;
        let output = transform_bunfig_for_env(input, false).unwrap().unwrap();
        assert!(output.starts_with("# Bun settings\n"));
        assert!(output.contains("url = 'https://us-npm.pkg.dev/project/repo'"));
        assert!(output.contains("extra = true"));
        assert!(output.contains("token = \"$NODE_AUTH_TOKEN\""));
        assert!(output.contains("# keep"));
        assert!(!output.contains("username"));
        assert!(!output.contains("password"));
        assert!(output.contains("other = \"https://registry.example.com/private\""));
    }

    #[test]
    fn converts_string_entry_to_inline_table() {
        let input = "[install.scopes]\nmyorg = 'https://us-npm.pkg.dev/project/repo'\n";
        let output = transform_bunfig_for_env(input, false).unwrap().unwrap();
        assert!(output.contains("url = 'https://us-npm.pkg.dev/project/repo'"));
        assert!(output.contains("token = \"$NODE_AUTH_TOKEN\""));
    }

    #[test]
    fn rejects_invalid_registry_shapes() {
        let error = registry_urls("[install.scopes]\nmyorg = 42\n", false).unwrap_err();
        assert!(error.to_string().contains("install.scopes.myorg"));
        assert!(registry_urls("[install\n", false).is_err());
    }

    #[test]
    fn domain_override_updates_other_https_registries() {
        let input = "[install.scopes]\nmyorg = \"https://registry.example.com/private\"\n";
        assert!(transform_bunfig_for_env(input, false).unwrap().is_none());
        let output = transform_bunfig_for_env(input, true).unwrap().unwrap();
        assert!(output.contains("token = \"$NODE_AUTH_TOKEN\""));
    }

    #[test]
    fn updates_env_token_once_and_preserves_line_endings() {
        let input =
            "# local\r\nNODE_AUTH_TOKEN=old\r\nOTHER=value\r\nexport NODE_AUTH_TOKEN=duplicate\r\n";
        let output = transform_env_local(input, "fresh-token").unwrap();
        assert_eq!(output.matches("NODE_AUTH_TOKEN=").count(), 1);
        assert!(output.contains("NODE_AUTH_TOKEN=fresh-token\r\n"));
        assert!(output.starts_with("# local\r\n"));
        assert!(output.contains("OTHER=value\r\n"));
        assert!(!output.contains("old"));
        assert!(!output.contains("duplicate"));
    }
}
