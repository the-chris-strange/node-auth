//! `.npmrc` configuration parsing, formatting, and transformation.
//!
//! Handles scoped and unscoped Artifact Registry repository definitions, `_authToken` credential lines,
//! legacy basic auth password and username cleanup, and comment/formatting preservation.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use regex::Regex;
use crate::error::AuthError;

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
        let (reg_pattern, auth_pattern, pass_pattern, user_pattern) = if allow_all_domains {
            (
                r"^(@[a-zA-Z0-9-*~][a-zA-Z0-9-*._~]*:)?registry=https:(//[^ \t\r\n]+/?)$",
                r"^(//[^ \t\r\n]+/?):_authToken=.*$",
                r"^(//[^ \t\r\n]+/?):_password=.*$",
                r"^(//[^ \t\r\n]+/?):username=oauth2accesstoken$",
            )
        } else {
            (
                r"^(@[a-zA-Z0-9-*~][a-zA-Z0-9-*._~]*:)?registry=https:(//[a-zA-Z0-9-]+[-]npm[.]pkg[.]dev/[^ \t\r\n]+/?)$",
                r"^(//[a-zA-Z0-9-]+[-]npm[.]pkg[.]dev/[^ \t\r\n]+/?):_authToken=.*$",
                r"^(//[a-zA-Z0-9-]+[-]npm[.]pkg[.]dev/[^ \t\r\n]+/?):_password=.*$",
                r"^(//[a-zA-Z0-9-]+[-]npm[.]pkg[.]dev/[^ \t\r\n]+/?):username=oauth2accesstoken$",
            )
        };

        Ok(Self {
            registry_re: Regex::new(reg_pattern)?,
            auth_token_re: Regex::new(auth_pattern)?,
            password_re: Regex::new(pass_pattern)?,
            username_re: Regex::new(user_pattern)?,
        })
    }

    /// Normalizes registry URL to always start with `//` and end with `/`.
    pub fn normalize_registry(reg: &str) -> String {
        let mut s = reg.trim().to_string();
        if !s.starts_with("//") {
            if let Some(stripped) = s.strip_prefix("https://") {
                s = format!("//{stripped}");
            } else if let Some(stripped) = s.strip_prefix("http://") {
                s = format!("//{stripped}");
            } else {
                s = format!("//{s}");
            }
        }
        if !s.ends_with('/') {
            s.push('/');
        }
        s
    }

    /// Parses a single `.npmrc` configuration line into a [`NpmrcConfigType`].
    pub fn parse_line(&self, line: &str) -> NpmrcConfigType {
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            return NpmrcConfigType::Other(line.to_string());
        }

        if let Some(caps) = self.registry_re.captures(trimmed) {
            let scope = caps.get(1).map(|m| {
                m.as_str().trim_end_matches(':').to_string()
            });
            let registry_raw = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            let registry = Self::normalize_registry(registry_raw);
            return NpmrcConfigType::Registry {
                scope,
                registry,
                raw: line.to_string(),
            };
        }

        if let Some(caps) = self.auth_token_re.captures(trimmed) {
            let registry = Self::normalize_registry(caps.get(1).map(|m| m.as_str()).unwrap_or(""));
            let token_part = trimmed
                .split_once(":_authToken=")
                .map(|(_, t)| t.trim().trim_matches('"').trim_matches('\'').to_string())
                .unwrap_or_default();
            return NpmrcConfigType::AuthToken {
                registry,
                token: token_part,
            };
        }

        if let Some(caps) = self.password_re.captures(trimmed) {
            let registry = Self::normalize_registry(caps.get(1).map(|m| m.as_str()).unwrap_or(""));
            let pass_part = trimmed
                .split_once(":_password=")
                .map(|(_, p)| p.trim().to_string())
                .unwrap_or_default();
            return NpmrcConfigType::Password {
                registry,
                password: pass_part,
            };
        }

        if let Some(caps) = self.username_re.captures(trimmed) {
            let registry = Self::normalize_registry(caps.get(1).map(|m| m.as_str()).unwrap_or(""));
            return NpmrcConfigType::Username { registry };
        }

        NpmrcConfigType::Other(line.to_string())
    }
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
    let parser = NpmrcParser::new(allow_all_domains)?;
    let mut registries_found = BTreeMap::new();
    let mut from_lines_out = Vec::new();

    // Parse source config
    for line in from_content.lines() {
        match parser.parse_line(line) {
            NpmrcConfigType::Registry { scope, registry, raw } => {
                registries_found.insert(registry.clone(), scope);
                from_lines_out.push(raw);
            }
            NpmrcConfigType::AuthToken { registry, .. } => {
                if same_file {
                    // Updating same file: will be refreshed below
                } else {
                    log::debug!("Moving existing _authToken for {registry} from project .npmrc to credential .npmrc");
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
            if let NpmrcConfigType::Registry { scope, registry, .. } = parser.parse_line(line) {
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
            let parsed = parser.parse_line(line);
            match parsed {
                NpmrcConfigType::AuthToken { registry, .. } => {
                    if pending_registries.contains_key(&registry) {
                        to_lines_out.push(format!("{registry}:_authToken=\"{creds}\""));
                        pending_registries.remove(&registry);
                    } else {
                        to_lines_out.push(line.to_string());
                    }
                }
                NpmrcConfigType::Password { registry, .. } => {
                    if pending_registries.contains_key(&registry) {
                        to_lines_out.push(format!("{registry}:_authToken=\"{creds}\""));
                        pending_registries.remove(&registry);
                    } else {
                        to_lines_out.push(line.to_string());
                    }
                }
                NpmrcConfigType::Username { registry } => {
                    if !pending_registries.contains_key(&registry) {
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
/// * `verbose` - If true, enables debug logging during configuration updates.
///
/// # Errors
///
/// Returns [`AuthError::Io`] if reading or writing files fails, or [`AuthError::Config`] if no registry was found.
pub fn update_npmrc_configs(
    from_path: &Path,
    to_path: &Path,
    creds: &str,
    allow_all_domains: bool,
    verbose: bool,
) -> Result<(), AuthError> {
    if verbose {
        crate::logger::set_verbose(true);
    }

    let same_file = from_path == to_path;

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

    let (from_output, to_output) = transform_npmrc_contents(
        &from_content,
        if same_file { &from_content } else { &to_content },
        creds,
        allow_all_domains,
        same_file,
    )?;

    // Ensure parent directory for to_path exists
    if let Some(parent) = to_path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }

    if same_file {
        fs::write(to_path, to_output)?;
        log::debug!("Updated local .npmrc in {}", to_path.display());
    } else {
        fs::write(to_path, to_output)?;
        log::debug!("Updated credential .npmrc in {}", to_path.display());

        if from_path.exists() {
            fs::write(from_path, from_output)?;
            log::debug!("Cleaned project .npmrc in {}", from_path.display());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_parser_standard_ar() {
        let parser = NpmrcParser::new(false).unwrap();

        let line = "@my-org:registry=https://us-central1-npm.pkg.dev/my-project/my-repo/";
        let parsed = parser.parse_line(line);
        match parsed {
            NpmrcConfigType::Registry { scope, registry, .. } => {
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
        let parsed = parser.parse_line(line);
        match parsed {
            NpmrcConfigType::Registry { scope, registry, .. } => {
                assert_eq!(scope, None);
                assert_eq!(registry, "//europe-west1-npm.pkg.dev/google-cloud/demo-repo/");
            }
            _ => panic!("Expected Registry variant"),
        }
    }

    #[test]
    fn test_parser_existing_auth_token() {
        let parser = NpmrcParser::new(false).unwrap();

        let line = "//us-central1-npm.pkg.dev/my-project/my-repo/:_authToken=\"ya29.sample-token\"";
        let parsed = parser.parse_line(line);
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
        let from = "@corp:registry=https://us-central1-npm.pkg.dev/my-corp/my-npm/\nstrict-ssl=true\n";
        let to = "//registry.npmjs.org/:_authToken=\"npm_secret_xyz\"\n";
        let token = "fresh-adc-token-12345";

        let (from_out, to_out) = transform_npmrc_contents(from, to, token, false, false).unwrap();
        assert!(from_out.contains("@corp:registry=https://us-central1-npm.pkg.dev/my-corp/my-npm/"));
        assert!(from_out.contains("strict-ssl=true"));
        assert!(to_out.contains("//registry.npmjs.org/:_authToken=\"npm_secret_xyz\""));
        assert!(to_out.contains("//us-central1-npm.pkg.dev/my-corp/my-npm/:_authToken=\"fresh-adc-token-12345\""));
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
        ).unwrap();

        fs::write(
            &user_npmrc,
            "//registry.npmjs.org/:_authToken=\"npm_secret_xyz\"\n",
        ).unwrap();

        let token = "fresh-adc-token-12345";
        update_npmrc_configs(&project_npmrc, &user_npmrc, token, false, false).unwrap();

        let project_out = fs::read_to_string(&project_npmrc).unwrap();
        let user_out = fs::read_to_string(&user_npmrc).unwrap();

        assert!(project_out.contains("@corp:registry=https://us-central1-npm.pkg.dev/my-corp/my-npm/"));
        assert!(project_out.contains("strict-ssl=true"));
        assert!(!project_out.contains("_authToken"));

        assert!(user_out.contains("//registry.npmjs.org/:_authToken=\"npm_secret_xyz\""));
        assert!(user_out.contains("//us-central1-npm.pkg.dev/my-corp/my-npm/:_authToken=\"fresh-adc-token-12345\""));
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
        ).unwrap();

        update_npmrc_configs(&project_npmrc, &user_npmrc, "new-token", false, false).unwrap();

        let project_out = fs::read_to_string(&project_npmrc).unwrap();
        let user_out = fs::read_to_string(&user_npmrc).unwrap();

        assert!(!project_out.contains("_password"));
        assert!(!project_out.contains("username=oauth2accesstoken"));
        assert!(user_out.contains("//us-central1-npm.pkg.dev/my-corp/my-npm/:_authToken=\"new-token\""));
    }

    #[test]
    fn test_update_npmrc_single_file_local_credential() {
        let dir = tempdir().unwrap();
        let local_npmrc = dir.path().join(".npmrc");

        fs::write(
            &local_npmrc,
            "@corp:registry=https://us-central1-npm.pkg.dev/my-corp/my-npm/\n\
             save-exact=true\n",
        ).unwrap();

        let token = "local-token-999";
        update_npmrc_configs(&local_npmrc, &local_npmrc, token, false, false).unwrap();

        let out = fs::read_to_string(&local_npmrc).unwrap();
        assert!(out.contains("@corp:registry=https://us-central1-npm.pkg.dev/my-corp/my-npm/"));
        assert!(out.contains("save-exact=true"));
        assert!(out.contains("//us-central1-npm.pkg.dev/my-corp/my-npm/:_authToken=\"local-token-999\""));
    }
}