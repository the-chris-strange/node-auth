//! # `node-auth`
//!
//! A fast, lightweight, standalone Rust CLI and library that authenticates Node package managers
//! (**npm**, **yarn**, and **pnpm**) to private **Google Artifact Registry (GAR)** npm repositories
//! using **Google Cloud Application Default Credentials (ADC)**.
//!
//! ## Overview
//!
//! `node-auth` replicates and improves upon Google's `@google-cloud/artifact-registry-npm-tools`
//! without requiring Node, npm, or pnpm to be installed or configured beforehand.
//!
//! ### Key Features
//! - **Automatic Credential Discovery**: Searches for explicit tokens, Application Default Credentials (ADC),
//!   and falls back to `gcloud auth print-access-token`.
//! - **Credential Isolation**: Reads registry endpoints from project-level `.npmrc` and writes secrets to
//!   user-level `~/.npmrc` by default, preventing sensitive tokens from being checked into source control.
//! - **Git Safety Checks**: When `--local-credential` is used, checks `.gitignore` and emits warnings if `.npmrc`
//!   is not properly ignored.
//! - **Yarn Modern Support**: Detects and updates `npmScopes` authentication in `.yarnrc.yml`.
//!
//! ### Library Usage Example
//!
//! ```no_run
//! use node_auth::{run, Options};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), node_auth::AuthError> {
//!     let options = Options {
//!         token: Some("custom-token".to_string()),
//!         ..Default::default()
//!     };
//!     run(&options).await?;
//!     Ok(())
//! }
//! ```

#![warn(missing_docs)]

pub mod auth;
pub mod cli;
pub mod error;
pub mod fs;
pub mod logger;
pub mod npmrc;
pub mod registry;
pub mod token;
pub mod vcs;
pub mod yarnrc;

pub use error::AuthError;
use std::path::{Path, PathBuf};

/// Useful result of a library authentication run, with presentation left to callers.
pub enum RunOutcome {
    /// The requested token, with no configuration files modified.
    Token(String),
    /// Paths replaced, and the Git safety status for local credentials if applicable.
    Updated {
        /// Absolute paths successfully replaced.
        paths: Vec<PathBuf>,
        /// Git status of the local `.npmrc` when `local_credential` was requested.
        git_status: Option<vcs::GitStatus>,
        /// Existing credential files that were broadly readable before replacement.
        broadly_readable: Vec<PathBuf>,
    },
}

impl std::fmt::Debug for RunOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Token(_) => formatter.write_str("Token([redacted])"),
            Self::Updated {
                paths,
                git_status,
                broadly_readable,
            } => formatter
                .debug_struct("Updated")
                .field("paths", paths)
                .field("git_status", git_status)
                .field("broadly_readable", broadly_readable)
                .finish(),
        }
    }
}

/// Configuration options for configuring Node authentication.
#[derive(Clone, Default)]
pub struct Options {
    /// Path to the `.npmrc` file to read registry configs from.
    /// Defaults to project-level `.npmrc` if present, otherwise user-level `~/.npmrc`.
    pub repo_config: Option<PathBuf>,

    /// Path to the `.npmrc` file to write credentials to.
    /// Defaults to user-level `~/.npmrc`.
    pub credential_config: Option<PathBuf>,

    /// Path to the `.yarnrc.yml` file to read registry configs from.
    /// Defaults to project-level `.yarnrc.yml` if present, otherwise user-level `~/.yarnrc.yml`.
    pub repo_config_yarn: Option<PathBuf>,

    /// Path to the `.yarnrc.yml` file to write credentials to.
    /// Defaults to user-level `~/.yarnrc.yml`.
    pub credential_config_yarn: Option<PathBuf>,

    /// Forces writing credentials to the current directory's `.npmrc`.
    pub local_credential: bool,

    /// Explicit OAuth2 access token to use instead of querying ADC or `gcloud`.
    pub token: Option<String>,

    /// Allow all registry domains to attach the auth token to (not only `*-npm.pkg.dev`).
    pub allow_all_domains: bool,

    /// Explicitly enable (`Some(true)`) or disable (`Some(false)`) updating `.yarnrc.yml`.
    /// When `None`, Yarn updating is auto-detected based on `.yarnrc.yml` presence.
    pub yarn: Option<bool>,

    /// Only retrieve and print the access token to standard output without modifying files.
    pub print_token: bool,
}

impl std::fmt::Debug for Options {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Options")
            .field("repo_config", &self.repo_config)
            .field("credential_config", &self.credential_config)
            .field("repo_config_yarn", &self.repo_config_yarn)
            .field("credential_config_yarn", &self.credential_config_yarn)
            .field("local_credential", &self.local_credential)
            .field("token", &self.token.as_ref().map(|_| "[redacted]"))
            .field("allow_all_domains", &self.allow_all_domains)
            .field("yarn", &self.yarn)
            .field("print_token", &self.print_token)
            .finish()
    }
}

/// Executes authentication and updates `.npmrc` and `.yarnrc.yml` configuration files.
///
/// # Arguments
///
/// * `options` - Parameters controlling file paths, tokens, and domain restrictions.
///
/// # Errors
///
/// Returns [`AuthError`] if authentication fails, files cannot be read/written,
/// or no Artifact Registry configuration is discovered.
pub async fn run(options: &Options) -> Result<RunOutcome, AuthError> {
    let token = auth::get_credentials(options.token.as_deref()).await?;
    token::validate_token(&token)?;
    if options.print_token {
        return Ok(RunOutcome::Token(token));
    }

    let home_dir = dirs::home_dir()
        .ok_or_else(|| AuthError::Config("Unable to determine user home directory".to_string()))?;

    // Determine npmrc paths
    let (repo_npmrc, cred_npmrc, git_status) = if options.local_credential {
        let cwd = std::env::current_dir()?;
        let git_status = vcs::check_git_status(&cwd);

        let local_npmrc = cwd.join(".npmrc");

        let repo = if let Some(ref r) = options.repo_config {
            r.clone()
        } else if local_npmrc.exists() {
            local_npmrc.clone()
        } else {
            home_dir.join(".npmrc")
        };

        (repo, local_npmrc, Some(git_status))
    } else {
        let repo = options.repo_config.clone().unwrap_or_else(|| {
            let local = Path::new(".npmrc");
            if local.exists() {
                local.to_path_buf()
            } else {
                home_dir.join(".npmrc")
            }
        });

        let cred = options
            .credential_config
            .clone()
            .unwrap_or_else(|| home_dir.join(".npmrc"));

        (repo, cred, None)
    };

    // Determine yarnrc paths
    let repo_yarn = options.repo_config_yarn.clone().unwrap_or_else(|| {
        let local = Path::new(".yarnrc.yml");
        if local.exists() {
            local.to_path_buf()
        } else {
            home_dir.join(".yarnrc.yml")
        }
    });

    let cred_yarn = options
        .credential_config_yarn
        .clone()
        .unwrap_or_else(|| home_dir.join(".yarnrc.yml"));

    let should_update_yarn = match options.yarn {
        Some(explicit) => explicit,
        None => {
            options.repo_config_yarn.is_some()
                || options.credential_config_yarn.is_some()
                || Path::new(".yarnrc.yml").exists()
        }
    };

    let repo_npmrc = fs::resolve(&repo_npmrc)?;
    let cred_npmrc = fs::resolve(&cred_npmrc)?;
    let repo_yarn = if should_update_yarn {
        Some(fs::resolve(&repo_yarn)?)
    } else {
        None
    };
    let cred_yarn = if should_update_yarn {
        Some(fs::resolve(&cred_yarn)?)
    } else {
        None
    };

    let mut paths = vec![repo_npmrc.clone(), cred_npmrc.clone()];
    if let Some(path) = &repo_yarn {
        paths.push(path.clone());
    }
    if let Some(path) = &cred_yarn {
        paths.push(path.clone());
    }
    let mut transaction = fs::FileTransaction::new(&paths)?;
    let mut broadly_readable = Vec::new();
    for credential_path in [&cred_npmrc, cred_yarn.as_ref().unwrap_or(&cred_npmrc)] {
        if transaction.is_broadly_readable(credential_path)?
            && !broadly_readable.contains(credential_path)
        {
            broadly_readable.push(credential_path.clone());
        }
    }

    let npm_source = transaction.contents(&repo_npmrc)?.to_owned();
    let npm_target = transaction.contents(&cred_npmrc)?.to_owned();
    if npmrc::has_registry(&npm_source, &npm_target, options.allow_all_domains)? {
        let same_file = repo_npmrc == cred_npmrc;
        let (cleaned, updated) = npmrc::transform_npmrc_contents(
            &npm_source,
            &npm_target,
            &token,
            options.allow_all_domains,
            same_file,
        )?;
        transaction.stage(&cred_npmrc, &updated)?;
        if !same_file && transaction.existed(&repo_npmrc)? && cleaned != npm_source {
            transaction.stage(&repo_npmrc, &cleaned)?;
        }
    } else if !should_update_yarn {
        return Err(AuthError::Config(
            "No Artifact Registry configuration found in .npmrc".to_string(),
        ));
    }

    if let (Some(source), Some(target)) = (repo_yarn, cred_yarn) {
        let source_content = transaction.contents(&source)?.to_owned();
        let target_content = transaction.contents(&target)?.to_owned();
        if let Some(updated) = yarnrc::transform_yarnrc_contents_with_policy(
            &source_content,
            &target_content,
            &token,
            options.allow_all_domains,
        )? {
            transaction.stage(&target, &updated)?;
        }
    }

    let paths = transaction.commit()?;
    if paths.is_empty() {
        return Err(AuthError::Config(
            "No eligible Artifact Registry configuration found".to_string(),
        ));
    }
    broadly_readable.retain(|path| paths.contains(path));
    Ok(RunOutcome::Updated {
        paths,
        git_status,
        broadly_readable,
    })
}
