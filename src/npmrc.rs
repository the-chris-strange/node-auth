//! `.npmrc` configuration parsing, formatting, and transformation.
//!
//! Handles scoped and unscoped Artifact Registry repository definitions, `_authToken` credential lines,
//! legacy basic auth password and username cleanup, and comment/formatting preservation.

use crate::error::AuthError;
use crate::fs::{FileTransaction, resolve};
use crate::registry::RegistryPolicy;
use crate::token::validate_token;
use regex::Regex;
use std::collections::BTreeMap;
use std::path::Path;

/// Recognized line configurations in `.npmrc` files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NpmrcConfigType {
    /// Registry URL specification (e.g. `@scope:registry=https://...` or `registry=https://...`).
    Registry {
        /// Optional npm package scope (e.g. `@my-org`).
        scope: Option<String>,
        /// Normalized registry URL (e.g. `//us-central1-npm.pkg.dev/proj/repo/`).
        registry: String,
        /// Original unmodified raw line content.
        raw: String,
    },
    /// Auth token credential line (e.g. `//...:_authToken="..."`).
    AuthToken {
        /// Normalized registry URL associated with this token.
        registry: String,
        /// Extracted authentication token string.
        token: String,
    },
    /// Legacy basic auth password line (e.g. `//...:_password=...`).
    Password {
        /// Normalized registry URL associated with this password.
        registry: String,
        /// Base64-encoded password value.
        password: String,
    },
    /// Legacy basic auth username line (e.g. `//...:username=oauth2accesstoken`).
    Username {
        /// Normalized registry URL.
        registry: String,
    },
    /// Comments, blank lines, or other unrecognized configuration keys.
    Other(String),
}

impl NpmrcConfigType {
    /// Formats the configuration entry back into an `.npmrc` line.
    pub fn to_line(&self) -> String {
        match self {
            NpmrcConfigType::Registry { raw, .. } => raw.clone(),
            NpmrcConfigType::AuthToken { registry, token } => {
                format!("{registry}:_authToken=\"{token}\"")
            }
            NpmrcConfigType::Password { registry, password } => {
                format!("{registry}:_password={password}")
            }
            NpmrcConfigType::Username { registry } => {
                format!("{registry}:username=oauth2accesstoken")
            }
            NpmrcConfigType::Other(line) => line.clone(),
        }
    }
}

/// Regular-expression-based parser for `.npmrc` lines.
pub struct NpmrcParser {
    policy: RegistryPolicy,
    registry_re: Regex,
    auth_token_re: Regex,
    password_re: Regex,
    username_re: Regex,
}

impl NpmrcParser {
    /// Creates a new parser instance.
    ///
    /// If `allow_all_domains` is false, only `*-npm.pkg.dev` Artifact Registry domains match.
    pub fn new(allow_all_domains: bool) -> Result<Self, AuthError> {
        Ok(Self {
            policy: RegistryPolicy { allow_all_domains },
            registry_re: Regex::new(
                r"^(@[a-zA-Z0-9-*~][a-zA-Z0-9-*._~]*:)?registry=(https://[^ \t\r\n]+)$",
            )?,
            auth_token_re: Regex::new(r"^(//[^ \t\r\n]+):_authToken=.*$")?,
            password_re: Regex::new(r"^(//[^ \t\r\n]+):_password=.*$")?,
            username_re: Regex::new(r"^(//[^ \t\r\n]+):username=oauth2accesstoken$")?,
        })
    }

    /// Parses a single `.npmrc` configuration line into a [`NpmrcConfigType`].
    pub fn parse_line(&self, line: &str) -> Result<NpmrcConfigType, AuthError> {
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            return Ok(NpmrcConfigType::Other(line.to_string()));
        }

        if let Some(caps) = self.registry_re.captures(trimmed) {
            let scope = caps
                .get(1)
                .map(|m| m.as_str().trim_end_matches(':').to_string());
            let registry_raw = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            if let Some(registry) = self.policy.parse(registry_raw)? {
                return Ok(NpmrcConfigType::Registry {
                    scope,
                    registry: registry.npm_key().to_string(),
                    raw: line.to_string(),
                });
            }
        }

        if let Some(caps) = self.auth_token_re.captures(trimmed) {
            let key = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            if let Some(registry) = self.policy.parse_npm_key(key)? {
                let token_part = trimmed
                    .split_once(":_authToken=")
                    .map(|(_, t)| t.trim().trim_matches('"').trim_matches('\'').to_string())
                    .unwrap_or_default();
                return Ok(NpmrcConfigType::AuthToken {
                    registry: registry.npm_key().to_string(),
                    token: token_part,
                });
            }
        }

        if let Some(caps) = self.password_re.captures(trimmed) {
            let key = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            if let Some(registry) = self.policy.parse_npm_key(key)? {
                let pass_part = trimmed
                    .split_once(":_password=")
                    .map(|(_, p)| p.trim().to_string())
                    .unwrap_or_default();
                return Ok(NpmrcConfigType::Password {
                    registry: registry.npm_key().to_string(),
                    password: pass_part,
                });
            }
        }

        if let Some(caps) = self.username_re.captures(trimmed) {
            let key = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            if let Some(registry) = self.policy.parse_npm_key(key)? {
                return Ok(NpmrcConfigType::Username {
                    registry: registry.npm_key().to_string(),
                });
            }
        }

        Ok(NpmrcConfigType::Other(line.to_string()))
    }
}

/// Whether either npm configuration contains an eligible registry definition.
pub fn has_registry(
    from_content: &str,
    to_content: &str,
    allow_all_domains: bool,
) -> Result<bool, AuthError> {
    let parser = NpmrcParser::new(allow_all_domains)?;
    for line in from_content.lines().chain(to_content.lines()) {
        if matches!(parser.parse_line(line)?, NpmrcConfigType::Registry { .. }) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Pure in-memory transformation of `.npmrc` contents.
///
/// Strips out legacy passwords and usernames, strips sensitive tokens from project configuration
/// when writing to user configuration, and inserts the fresh auth token for each Artifact Registry scope.
///
/// # Arguments
///
/// * `from_content` - Raw text of the source repository `.npmrc`.
/// * `to_content` - Raw text of the target credential `.npmrc`.
/// * `creds` - The OAuth2 access token to insert.
/// * `allow_all_domains` - Whether non-GAR domains are allowed for authentication.
/// * `same_file` - True if reading and writing to the exact same file (e.g. `--local-credential`).
///
/// # Returns
///
/// Returns `(cleaned_from_content, updated_to_content)`.
///
/// # Errors
///
/// Returns [`AuthError::Config`] if no Artifact Registry configurations were found in the input.
pub fn transform_npmrc_contents(
    from_content: &str,
    to_content: &str,
    creds: &str,
    allow_all_domains: bool,
    same_file: bool,
) -> Result<(String, String), AuthError> {
    validate_token(creds)?;
    let parser = NpmrcParser::new(allow_all_domains)?;
    let mut registries_found = BTreeMap::new();
    let mut from_lines_out = Vec::new();

    // Parse source config
    for line in from_content.lines() {
        match parser.parse_line(line)? {
            NpmrcConfigType::Registry {
                scope,
                registry,
                raw,
            } => {
                registries_found.insert(registry.clone(), scope);
                from_lines_out.push(raw);
            }
            NpmrcConfigType::AuthToken { registry, .. } => {
                if same_file {
                    // Updating same file: will be refreshed below
                } else {
                    log::debug!(
                        "Moving existing _authToken for {registry} from project .npmrc to credential .npmrc"
                    );
                    // Strip from project config so secrets aren't checked into git
                }
            }
            NpmrcConfigType::Password { registry, .. } => {
                if !same_file {
                    log::debug!("Removing legacy password for {registry} from project .npmrc");
                }
            }
            NpmrcConfigType::Username { .. } => {
                // Strip legacy oauth username line
            }
            NpmrcConfigType::Other(other) => {
                from_lines_out.push(other);
            }
        }
    }

    if !same_file {
        for line in to_content.lines() {
            if let NpmrcConfigType::Registry {
                scope, registry, ..
            } = parser.parse_line(line)?
            {
                registries_found.entry(registry).or_insert(scope);
            }
        }
    }

    if registries_found.is_empty() {
        return Err(AuthError::Config(
            "No Artifact Registry configuration found.\n\
             Ensure your .npmrc contains a registry line such as:\n\
             @my-scope:registry=https://<region>-npm.pkg.dev/<project>/<repo>/\n\
             Run `gcloud artifacts print-settings npm` to generate configuration."
                .to_string(),
        ));
    }

    // Prepare credentials to write to to_content
    let mut to_lines_out = Vec::new();
    let mut pending_registries = registries_found.clone();

    if !to_content.is_empty() {
        for line in to_content.lines() {
            let parsed = parser.parse_line(line)?;
            match parsed {
                NpmrcConfigType::AuthToken { registry, .. } => {
                    if registries_found.contains_key(&registry) {
                        if pending_registries.remove(&registry).is_some() {
                            to_lines_out.push(format!("{registry}:_authToken=\"{creds}\""));
                        }
                    } else {
                        to_lines_out.push(line.to_string());
                    }
                }
                NpmrcConfigType::Password { registry, .. } => {
                    if registries_found.contains_key(&registry) {
                        if pending_registries.remove(&registry).is_some() {
                            to_lines_out.push(format!("{registry}:_authToken=\"{creds}\""));
                        }
                    } else {
                        to_lines_out.push(line.to_string());
                    }
                }
                NpmrcConfigType::Username { registry } => {
                    if !registries_found.contains_key(&registry) {
                        to_lines_out.push(line.to_string());
                    }
                }
                _ => {
                    to_lines_out.push(line.to_string());
                }
            }
        }
    }

    // Append auth tokens for registries not yet present in to_content
    for (registry, _) in pending_registries {
        log::debug!("Adding auth token for registry {registry}");
        to_lines_out.push(format!("{registry}:_authToken=\"{creds}\""));
    }

    let to_output = to_lines_out.join("\n") + "\n";
    let from_output = from_lines_out.join("\n") + "\n";

    Ok((from_output, to_output))
}

/// Updates `.npmrc` configuration files with the given access token.
///
/// # Arguments
///
/// * `from_path` - Path to `.npmrc` file to read registry settings from (e.g. project `.npmrc`).
/// * `to_path` - Path to `.npmrc` file to write credentials to (e.g. user `~/.npmrc` or local `./.npmrc`).
/// * `creds` - The OAuth2 access token string.
/// * `allow_all_domains` - Whether to allow non-pkg.dev domains.
/// # Errors
///
/// Returns [`AuthError::Io`] if reading or writing files fails, or [`AuthError::Config`] if no registry was found.
pub fn update_npmrc_configs(
    from_path: &Path,
    to_path: &Path,
    creds: &str,
    allow_all_domains: bool,
) -> Result<(), AuthError> {
    let from_path = resolve(from_path)?;
    let to_path = resolve(to_path)?;
    let same_file = from_path == to_path;
    let mut transaction = FileTransaction::new(&[from_path.clone(), to_path.clone()])?;
    let from_content = transaction.contents(&from_path)?.to_owned();
    let to_content = transaction.contents(&to_path)?.to_owned();

    let (from_output, to_output) = transform_npmrc_contents(
        &from_content,
        if same_file {
            &from_content
        } else {
            &to_content
        },
        creds,
        allow_all_domains,
        same_file,
    )?;

    transaction.stage(&to_path, &to_output)?;
    if !same_file && transaction.existed(&from_path)? && from_output != from_content {
        transaction.stage(&from_path, &from_output)?;
    }
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_parser_standard_ar() {
        let parser = NpmrcParser::new(false).unwrap();

        let line = "@my-org:registry=https://us-central1-npm.pkg.dev/my-project/my-repo/";
        let parsed = parser.parse_line(line).unwrap();
        match parsed {
            NpmrcConfigType::Registry {
                scope, registry, ..
            } => {
                assert_eq!(scope.as_deref(), Some("@my-org"));
                assert_eq!(registry, "//us-central1-npm.pkg.dev/my-project/my-repo/");
            }
            _ => panic!("Expected Registry variant"),
        }
    }

    #[test]
    fn test_parser_unscoped_ar() {
        let parser = NpmrcParser::new(false).unwrap();

        let line = "registry=https://europe-west1-npm.pkg.dev/google-cloud/demo-repo/";
        let parsed = parser.parse_line(line).unwrap();
        match parsed {
            NpmrcConfigType::Registry {
                scope, registry, ..
            } => {
                assert_eq!(scope, None);
                assert_eq!(
                    registry,
                    "//europe-west1-npm.pkg.dev/google-cloud/demo-repo/"
                );
            }
            _ => panic!("Expected Registry variant"),
        }
    }

    #[test]
    fn test_parser_existing_auth_token() {
        let parser = NpmrcParser::new(false).unwrap();

        let line = "//us-central1-npm.pkg.dev/my-project/my-repo/:_authToken=\"ya29.sample-token\"";
        let parsed = parser.parse_line(line).unwrap();
        match parsed {
            NpmrcConfigType::AuthToken { registry, token } => {
                assert_eq!(registry, "//us-central1-npm.pkg.dev/my-project/my-repo/");
                assert_eq!(token, "ya29.sample-token");
            }
            _ => panic!("Expected AuthToken variant"),
        }
    }

    #[test]
    fn test_in_memory_transform() {
        let from =
            "@corp:registry=https://us-central1-npm.pkg.dev/my-corp/my-npm/\nstrict-ssl=true\n";
        let to = "//registry.npmjs.org/:_authToken=\"npm_secret_xyz\"\n";
        let token = "fresh-adc-token-12345";

        let (from_out, to_out) = transform_npmrc_contents(from, to, token, false, false).unwrap();
        assert!(
            from_out.contains("@corp:registry=https://us-central1-npm.pkg.dev/my-corp/my-npm/")
        );
        assert!(from_out.contains("strict-ssl=true"));
        assert!(to_out.contains("//registry.npmjs.org/:_authToken=\"npm_secret_xyz\""));
        assert!(to_out.contains(
            "//us-central1-npm.pkg.dev/my-corp/my-npm/:_authToken=\"fresh-adc-token-12345\""
        ));
    }

    #[test]
    fn duplicate_legacy_credentials_are_removed() {
        let source = "@corp:registry=https://us-npm.pkg.dev/project/repo/\n";
        let target = "//us-npm.pkg.dev/project/repo/:_authToken=old\n//us-npm.pkg.dev/project/repo/:_password=old\n//us-npm.pkg.dev/project/repo/:username=oauth2accesstoken\n";
        let (_, updated) = transform_npmrc_contents(source, target, "new", false, false).unwrap();
        assert_eq!(updated.matches(":_authToken=").count(), 1);
        assert!(!updated.contains("old"));
        assert!(!updated.contains(":username="));
    }

    #[test]
    fn test_update_npmrc_moves_token_to_user_config() {
        let dir = tempdir().unwrap();
        let project_npmrc = dir.path().join("project.npmrc");
        let user_npmrc = dir.path().join("user.npmrc");

        fs::write(
            &project_npmrc,
            "# Project configuration\n\
             @corp:registry=https://us-central1-npm.pkg.dev/my-corp/my-npm/\n\
             //us-central1-npm.pkg.dev/my-corp/my-npm/:_authToken=\"old-expired-token\"\n\
             strict-ssl=true\n",
        )
        .unwrap();

        fs::write(
            &user_npmrc,
            "//registry.npmjs.org/:_authToken=\"npm_secret_xyz\"\n",
        )
        .unwrap();

        let token = "fresh-adc-token-12345";
        update_npmrc_configs(&project_npmrc, &user_npmrc, token, false).unwrap();

        let project_out = fs::read_to_string(&project_npmrc).unwrap();
        let user_out = fs::read_to_string(&user_npmrc).unwrap();

        assert!(
            project_out.contains("@corp:registry=https://us-central1-npm.pkg.dev/my-corp/my-npm/")
        );
        assert!(project_out.contains("strict-ssl=true"));
        assert!(!project_out.contains("_authToken"));

        assert!(user_out.contains("//registry.npmjs.org/:_authToken=\"npm_secret_xyz\""));
        assert!(user_out.contains(
            "//us-central1-npm.pkg.dev/my-corp/my-npm/:_authToken=\"fresh-adc-token-12345\""
        ));
    }

    #[test]
    fn test_update_npmrc_legacy_password_conversion() {
        let dir = tempdir().unwrap();
        let project_npmrc = dir.path().join("project.npmrc");
        let user_npmrc = dir.path().join("user.npmrc");

        fs::write(
            &project_npmrc,
            "@corp:registry=https://us-central1-npm.pkg.dev/my-corp/my-npm/\n\
             //us-central1-npm.pkg.dev/my-corp/my-npm/:_password=base64pass\n\
             //us-central1-npm.pkg.dev/my-corp/my-npm/:username=oauth2accesstoken\n",
        )
        .unwrap();

        update_npmrc_configs(&project_npmrc, &user_npmrc, "new-token", false).unwrap();

        let project_out = fs::read_to_string(&project_npmrc).unwrap();
        let user_out = fs::read_to_string(&user_npmrc).unwrap();

        assert!(!project_out.contains("_password"));
        assert!(!project_out.contains("username=oauth2accesstoken"));
        assert!(
            user_out.contains("//us-central1-npm.pkg.dev/my-corp/my-npm/:_authToken=\"new-token\"")
        );
    }

    #[test]
    fn test_update_npmrc_single_file_local_credential() {
        let dir = tempdir().unwrap();
        let local_npmrc = dir.path().join(".npmrc");

        fs::write(
            &local_npmrc,
            "@corp:registry=https://us-central1-npm.pkg.dev/my-corp/my-npm/\n\
             save-exact=true\n",
        )
        .unwrap();

        let token = "local-token-999";
        update_npmrc_configs(&local_npmrc, &local_npmrc, token, false).unwrap();

        let out = fs::read_to_string(&local_npmrc).unwrap();
        assert!(out.contains("@corp:registry=https://us-central1-npm.pkg.dev/my-corp/my-npm/"));
        assert!(out.contains("save-exact=true"));
        assert!(
            out.contains(
                "//us-central1-npm.pkg.dev/my-corp/my-npm/:_authToken=\"local-token-999\""
            )
        );
    }
}
