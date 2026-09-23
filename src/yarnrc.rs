//! Yarn Berry / Yarn Modern (`.yarnrc.yml`) configuration management.
//!
//! Handles parsing, updating, and writing `npmScopes` authentication entries
//! (`npmAlwaysAuth` and `npmAuthToken`) in `.yarnrc.yml` files.

use crate::error::AuthError;
use crate::fs::{FileTransaction, resolve};
use crate::registry::RegistryPolicy;
use crate::token::validate_token;
use std::path::Path;
use std::str::FromStr;
use yaml_edit::{Mapping, YamlFile};

/// Transforms Yarn Modern (`.yarnrc.yml`) content in memory to insert authentication tokens for detected scopes.
///
/// # Arguments
///
/// * `from_content` - Raw YAML text of the source/repository `.yarnrc.yml`.
/// * `to_content` - Raw YAML text of the destination `.yarnrc.yml` (e.g. user-level config).
/// * `creds` - The OAuth2 access token to insert into `npmAuthToken`.
///
/// # Returns
///
/// * `Ok(Some(updated_yaml))` if any `npmScopes` were found and updated.
/// * `Ok(None)` if no scopes were found.
///
/// # Errors
///
/// Returns [`AuthError::YamlInput`] if either input is invalid YAML.
pub fn transform_yarnrc_contents(
    from_content: &str,
    to_content: &str,
    creds: &str,
) -> Result<Option<String>, AuthError> {
    transform_yarnrc_contents_with_policy(from_content, to_content, creds, false)
}

/// Transform Yarn configuration using the same registry policy as npm.
pub fn transform_yarnrc_contents_with_policy(
    from_content: &str,
    to_content: &str,
    creds: &str,
    allow_all_domains: bool,
) -> Result<Option<String>, AuthError> {
    validate_token(creds)?;
    let from_file = parse_yarnrc(from_content, "source .yarnrc.yml")?;
    let to_file = parse_yarnrc(to_content, "credential .yarnrc.yml")?;
    let from_doc = from_file.document().ok_or_else(|| {
        AuthError::Config("Yarn configuration must contain a document".to_string())
    })?;
    let to_doc = to_file.document().ok_or_else(|| {
        AuthError::Config("Yarn credential configuration must contain a document".to_string())
    })?;
    let policy = RegistryPolicy { allow_all_domains };
    let from_root = from_doc.as_mapping().ok_or_else(|| {
        AuthError::Config("Yarn configuration must be a YAML mapping".to_string())
    })?;
    let to_root = to_doc.as_mapping().ok_or_else(|| {
        AuthError::Config("Yarn credential configuration must be a YAML mapping".to_string())
    })?;
    let mut found_any = false;

    if let Some(from_scopes_value) = from_root.get("npmScopes") {
        let from_scopes = from_scopes_value
            .as_mapping()
            .ok_or_else(|| AuthError::Config("Yarn npmScopes must be a mapping".to_string()))?;
        for (scope_key, scope_val) in from_scopes.iter() {
            let scope_name = scope_key
                .as_scalar()
                .map(|key| key.as_string())
                .ok_or_else(|| AuthError::Config("Yarn scope name must be a string".to_string()))?;
            let scope = scope_val.as_mapping().ok_or_else(|| {
                AuthError::Config(format!("Yarn scope `{scope_name}` must be a mapping"))
            })?;
            let Some(raw_registry) = scope.get("npmRegistryServer") else {
                continue;
            };
            let registry = raw_registry
                .as_scalar()
                .map(|value| value.as_string())
                .ok_or_else(|| {
                    AuthError::Config(format!(
                        "Yarn scope `{scope_name}` registry must be a string"
                    ))
                })?;
            if policy.parse(&registry)?.is_none() {
                continue;
            }
            found_any = true;

            let target_scopes = ensure_mapping(&to_root, "npmScopes").ok_or_else(|| {
                AuthError::Config("Destination Yarn npmScopes must be a mapping".to_string())
            })?;
            let target_scope = ensure_mapping(&target_scopes, &scope_name).ok_or_else(|| {
                AuthError::Config(format!(
                    "Destination Yarn scope `{scope_name}` must be a mapping"
                ))
            })?;
            target_scope.set("npmRegistryServer", registry);
            target_scope.set("npmAlwaysAuth", true);
            target_scope.set("npmAuthToken", creds);
        }
    }

    if !found_any {
        return Ok(None);
    }
    Ok(Some(to_file.to_string()))
}

fn parse_yarnrc(content: &str, input: &'static str) -> Result<YamlFile, AuthError> {
    if content.trim().is_empty() {
        let file = YamlFile::new();
        file.ensure_document();
        Ok(file)
    } else {
        YamlFile::from_str(content).map_err(|source| AuthError::YamlInput { input, source })
    }
}

fn ensure_mapping(parent: &Mapping, key: &str) -> Option<Mapping> {
    match parent.get(key) {
        Some(value) => value.as_mapping().cloned(),
        None => {
            parent.set(key, Mapping::new_pending_block());
            parent.get_mapping(key)
        }
    }
}

/// Updates `.yarnrc.yml` files with the access token.
///
/// # Arguments
///
/// * `from_path` - Path to project `.yarnrc.yml` to read scope configurations from.
/// * `to_path` - Path to user `.yarnrc.yml` to write credentials to.
/// * `creds` - Google OAuth2 access token string.
/// # Errors
///
/// Returns [`AuthError::Io`] if reading or writing files fails, or
/// [`AuthError::YamlInput`] if YAML parsing fails.
pub fn update_yarn_configs(from_path: &Path, to_path: &Path, creds: &str) -> Result<(), AuthError> {
    update_yarn_configs_with_policy(from_path, to_path, creds, false)
}

/// Update Yarn configuration with an explicit registry-domain policy.
pub fn update_yarn_configs_with_policy(
    from_path: &Path,
    to_path: &Path,
    creds: &str,
    allow_all_domains: bool,
) -> Result<(), AuthError> {
    let from_path = resolve(from_path)?;
    let to_path = resolve(to_path)?;
    let mut transaction = FileTransaction::new(&[from_path.clone(), to_path.clone()])?;
    let from_content = transaction.contents(&from_path)?.to_owned();
    let to_content = transaction.contents(&to_path)?.to_owned();
    match transform_yarnrc_contents_with_policy(
        &from_content,
        &to_content,
        creds,
        allow_all_domains,
    )? {
        Some(dumped) => {
            transaction.stage(&to_path, &dumped)?;
            transaction.commit()?;
            Ok(())
        }
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_in_memory_yarnrc_transform() {
        let from = r#"
npmScopes:
  my-scope:
    npmRegistryServer: "https://us-central1-npm.pkg.dev/my-project/my-repo"
"#;
        let res = transform_yarnrc_contents(from, "", "test-token").unwrap();
        assert!(res.is_some());
        let out = res.unwrap();
        assert!(out.contains("npmScopes"));
        assert!(out.contains("my-scope"));
        assert!(out.contains("npmAuthToken: test-token"));
    }

    #[test]
    fn transform_preserves_unrelated_formatting_and_comments() {
        let from = r#"npmScopes:
  my-scope:
    npmRegistryServer: "https://us-central1-npm.pkg.dev/my-project/my-repo"
"#;
        let to = r#"# User-level Yarn settings
nodeLinker: pnp # keep this inline comment

npmScopes:
  my-scope:
    npmRegistryServer: 'https://old.example/my-project/my-repo' # registry note
    customSetting: "keep-me"

checksumBehavior: 'throw'
"#;

        let out = transform_yarnrc_contents(from, to, "test-token")
            .unwrap()
            .unwrap();

        assert!(out.starts_with("# User-level Yarn settings\n"));
        assert!(out.contains("nodeLinker: pnp # keep this inline comment\n\n"));
        assert!(out.contains("# registry note\n"));
        assert!(out.contains("    customSetting: \"keep-me\"\n"));
        assert!(out.ends_with("\nchecksumBehavior: 'throw'\n"));
        assert!(out.contains("npmAlwaysAuth: true"));
        assert!(out.contains("npmAuthToken: test-token"));
    }

    #[test]
    fn transform_quotes_yaml_sensitive_tokens() {
        let from = r#"npmScopes:
  my-scope:
    npmRegistryServer: https://us-central1-npm.pkg.dev/my-project/my-repo
"#;

        let out = transform_yarnrc_contents(from, "", "token:#value")
            .unwrap()
            .unwrap();
        let parsed = YamlFile::from_str(&out).unwrap().document().unwrap();
        let token = parsed
            .get_mapping("npmScopes")
            .unwrap()
            .get_mapping("my-scope")
            .unwrap()
            .get("npmAuthToken")
            .unwrap()
            .as_scalar()
            .unwrap()
            .as_string();

        assert_eq!(token, "token:#value");
    }

    #[test]
    fn test_update_yarnrc_creates_scope_auth() {
        let dir = tempdir().unwrap();
        let project_yarn = dir.path().join("project.yarnrc.yml");
        let user_yarn = dir.path().join("user.yarnrc.yml");

        fs::write(
            &project_yarn,
            r#"
npmScopes:
  my-scope:
    npmRegistryServer: "https://us-central1-npm.pkg.dev/my-project/my-repo"
"#,
        )
        .unwrap();

        let token = "yarn-token-12345";
        update_yarn_configs(&project_yarn, &user_yarn, token).unwrap();

        let user_out = fs::read_to_string(&user_yarn).unwrap();
        assert!(user_out.contains("npmScopes"));
        assert!(user_out.contains("my-scope"));
        assert!(user_out.contains("https://us-central1-npm.pkg.dev/my-project/my-repo"));
        assert!(user_out.contains("npmAlwaysAuth: true"));
        assert!(user_out.contains("npmAuthToken: yarn-token-12345"));
    }
}
