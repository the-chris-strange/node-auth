//! Yarn Berry / Yarn Modern (`.yarnrc.yml`) configuration management.
//!
//! Handles parsing, updating, and writing `npmScopes` authentication entries
//! (`npmAlwaysAuth` and `npmAuthToken`) in `.yarnrc.yml` files.

use crate::error::AuthError;
use serde_yaml::{Mapping, Value};
use std::fs;
use std::path::Path;

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
/// Returns [`AuthError::Yaml`] if serialization fails.
pub fn transform_yarnrc_contents(
    from_content: &str,
    to_content: &str,
    creds: &str,
) -> Result<Option<String>, AuthError> {
    let mut from_doc: Value = if !from_content.is_empty() {
        serde_yaml::from_str(from_content).unwrap_or(Value::Mapping(Mapping::new()))
    } else {
        Value::Mapping(Mapping::new())
    };

    let mut to_doc: Value = if !to_content.is_empty() {
        serde_yaml::from_str(to_content).unwrap_or(Value::Mapping(Mapping::new()))
    } else {
        Value::Mapping(Mapping::new())
    };

    let mut found_any = false;

    if let Some(from_scopes) = from_doc
        .get_mut("npmScopes")
        .and_then(|v| v.as_mapping_mut())
    {
        for (scope_key, scope_val) in from_scopes.iter() {
            if let Some(registry) = scope_val.get("npmRegistryServer").and_then(|r| r.as_str()) {
                found_any = true;
                let scope_name = scope_key.as_str().unwrap_or("default");
                log::debug!("Found yarn scope '{scope_name}' with registry '{registry}'");

                if !to_doc.is_mapping() {
                    to_doc = Value::Mapping(Mapping::new());
                }

                let to_root = to_doc.as_mapping_mut().unwrap();
                let scopes_entry = to_root
                    .entry(Value::String("npmScopes".to_string()))
                    .or_insert_with(|| Value::Mapping(Mapping::new()));

                if let Some(to_scopes) = scopes_entry.as_mapping_mut() {
                    let target_scope = to_scopes
                        .entry(scope_key.clone())
                        .or_insert_with(|| Value::Mapping(Mapping::new()));

                    if let Some(target_map) = target_scope.as_mapping_mut() {
                        target_map.insert(
                            Value::String("npmRegistryServer".to_string()),
                            Value::String(registry.to_string()),
                        );
                        target_map.insert(
                            Value::String("npmAlwaysAuth".to_string()),
                            Value::Bool(true),
                        );
                        target_map.insert(
                            Value::String("npmAuthToken".to_string()),
                            Value::String(creds.to_string()),
                        );
                    }
                }
            }
        }
    }

    if !found_any {
        return Ok(None);
    }

    let dumped = serde_yaml::to_string(&to_doc)?;
    Ok(Some(dumped))
}

/// Updates `.yarnrc.yml` files with the access token.
///
/// # Arguments
///
/// * `from_path` - Path to project `.yarnrc.yml` to read scope configurations from.
/// * `to_path` - Path to user `.yarnrc.yml` to write credentials to.
/// * `creds` - Google OAuth2 access token string.
/// * `verbose` - If true, enables debug logging during file updates.
///
/// # Errors
///
/// Returns [`AuthError::Io`] if reading or writing files fails, or [`AuthError::Yaml`] if YAML parsing fails.
pub fn update_yarn_configs(
    from_path: &Path,
    to_path: &Path,
    creds: &str,
    verbose: bool,
) -> Result<(), AuthError> {
    if verbose {
        crate::logger::set_verbose(true);
    }

    let from_content = if from_path.exists() {
        fs::read_to_string(from_path)?
    } else {
        String::new()
    };

    let to_content = if to_path.exists() {
        fs::read_to_string(to_path)?
    } else {
        String::new()
    };

    match transform_yarnrc_contents(&from_content, &to_content, creds)? {
        Some(dumped) => {
            // Ensure parent directory exists
            if let Some(parent) = to_path.parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    fs::create_dir_all(parent)?;
                }
            }

            fs::write(to_path, dumped)?;
            log::info!("Updated yarn credentials in {}", to_path.display());
            Ok(())
        }
        None => {
            log::debug!(
                "No yarn npmScopes found in {}. Skipping yarn update.",
                from_path.display()
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        update_yarn_configs(&project_yarn, &user_yarn, token, false).unwrap();

        let user_out = fs::read_to_string(&user_yarn).unwrap();
        assert!(user_out.contains("npmScopes"));
        assert!(user_out.contains("my-scope"));
        assert!(user_out.contains("https://us-central1-npm.pkg.dev/my-project/my-repo"));
        assert!(user_out.contains("npmAlwaysAuth: true"));
        assert!(user_out.contains("npmAuthToken: yarn-token-12345"));
    }
}
