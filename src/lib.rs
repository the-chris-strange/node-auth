//! # `node-auth`
//!
//! A fast, lightweight, standalone Rust CLI and library that authenticates JavaScript package managers
//! (**npm**, **yarn**, **pnpm**, and **Bun**) to private **Google Artifact Registry (GAR)** npm repositories
//! using **Google Cloud Application Default Credentials (ADC)**.
//!
//! ## Overview
//!
//! `node-auth` replicates and improves upon Google's `@google-cloud/artifact-registry-npm-tools`
//! without requiring a JavaScript runtime or package manager to be installed beforehand.
//!
//! ### Key Features
//! - **Automatic Credential Discovery**: Searches for explicit tokens, Application Default Credentials (ADC),
//!   and falls back to `gcloud auth print-access-token`.
//! - **Credential Isolation**: Reads registry endpoints from project-level `.npmrc` and writes secrets to
//!   user-level `~/.npmrc` by default, preventing sensitive tokens from being checked into source control.
//! - **Git Safety Checks**: When `--local-credential` is used, checks `.gitignore` and emits warnings if `.npmrc`
//!   is not properly ignored.
//! - **Yarn Modern Support**: Detects and updates `npmScopes` authentication in `.yarnrc.yml`.
//! - **Bun Support**: Discovers registries in `bunfig.toml` and optionally manages a local
//!   `.env.local` token reference.
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
pub mod bunfig;
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
        /// Git safety status for `.env.local` when `bun_env` was requested.
        bun_env_git_status: Option<vcs::GitStatus>,
    },
}

impl std::fmt::Debug for RunOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Token(_) => formatter.write_str("Token([redacted])"),
            Self::Updated {
                paths,
                git_status,
                bun_env_git_status,
            } => formatter
                .debug_struct("Updated")
                .field("paths", paths)
                .field("git_status", git_status)
                .field("bun_env_git_status", bun_env_git_status)
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

    /// Path to the project `bunfig.toml` used to discover Bun registry configuration.
    /// Defaults to `./bunfig.toml`.
    pub repo_config_bun: Option<PathBuf>,

    /// Forces writing credentials to the current directory's `.npmrc`.
    pub local_credential: bool,

    /// Explicit OAuth2 access token to use instead of querying ADC or `gcloud`.
    pub token: Option<String>,

    /// Allow all HTTPS registry domains to receive the auth token (not only `*-npm.pkg.dev`).
    pub allow_all_domains: bool,

    /// Explicitly enable (`Some(true)`) or disable (`Some(false)`) updating `.yarnrc.yml`.
    /// When `None`, Yarn updating is auto-detected based on `.yarnrc.yml` presence.
    pub yarn: Option<bool>,

    /// Explicitly enable (`Some(true)`) or disable (`Some(false)`) reading `bunfig.toml`.
    /// When `None`, Bun support is auto-detected from `./bunfig.toml`.
    pub bun: Option<bool>,

    /// Write Bun's token to `.env.local` and reference it from the project `bunfig.toml`.
    pub bun_env: bool,

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
            .field("repo_config_bun", &self.repo_config_bun)
            .field("local_credential", &self.local_credential)
            .field("token", &self.token.as_ref().map(|_| "[redacted]"))
            .field("allow_all_domains", &self.allow_all_domains)
            .field("yarn", &self.yarn)
            .field("bun", &self.bun)
            .field("bun_env", &self.bun_env)
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
    if options.bun_env && options.local_credential {
        return Err(AuthError::Config(
            "--bun-env cannot be used with --local-credential".to_string(),
        ));
    }
    if options.bun_env && options.bun == Some(false) {
        return Err(AuthError::Config(
            "--bun-env cannot be used with --bun false".to_string(),
        ));
    }
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

    let repo_bun = options
        .repo_config_bun
        .clone()
        .unwrap_or_else(|| PathBuf::from("bunfig.toml"));
    let should_update_bun = options.bun_env
        || match options.bun {
            Some(explicit) => explicit,
            None => options.repo_config_bun.is_some() || Path::new("bunfig.toml").exists(),
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
    let repo_bun = if should_update_bun {
        Some(fs::resolve(&repo_bun)?)
    } else {
        None
    };
    let bun_env_path = if options.bun_env {
        let parent = repo_bun
            .as_ref()
            .and_then(|path| path.parent())
            .ok_or_else(|| AuthError::Config("bunfig.toml has no parent directory".to_string()))?;
        Some(fs::resolve(&parent.join(".env.local"))?)
    } else {
        None
    };
    let bun_env_git_status = bun_env_path
        .as_ref()
        .map(|path| vcs::check_path_status(path.parent().unwrap(), Path::new(".env.local")));

    let mut paths = vec![repo_npmrc.clone(), cred_npmrc.clone()];
    if let Some(path) = &repo_yarn {
        paths.push(path.clone());
    }
    if let Some(path) = &cred_yarn {
        paths.push(path.clone());
    }
    if let Some(path) = &repo_bun {
        paths.push(path.clone());
    }
    if let Some(path) = &bun_env_path {
        paths.push(path.clone());
    }
    let mut transaction = fs::FileTransaction::new(&paths)?;

    let bun_registries = if let Some(path) = &repo_bun {
        if !transaction.existed(path)? {
            return Err(AuthError::Config(format!(
                "Bun configuration does not exist: {}",
                path.display()
            )));
        }
        bunfig::registry_urls(transaction.contents(path)?, options.allow_all_domains)?
    } else {
        Vec::new()
    };

    let npm_source = transaction.contents(&repo_npmrc)?.to_owned();
    let npm_target = transaction.contents(&cred_npmrc)?.to_owned();
    let npm_has_registry =
        npmrc::has_registry(&npm_source, &npm_target, options.allow_all_domains)?;
    let bun_npm_registries = if options.local_credential || options.bun_env {
        &[][..]
    } else {
        bun_registries.as_slice()
    };
    if npm_has_registry || !bun_npm_registries.is_empty() {
        let same_file = repo_npmrc == cred_npmrc;
        let (cleaned, updated) = npmrc::transform_npmrc_contents_with_registries(
            &npm_source,
            &npm_target,
            &token,
            options.allow_all_domains,
            same_file,
            bun_npm_registries,
        )?;
        transaction.stage(&cred_npmrc, &updated)?;
        if !same_file && transaction.existed(&repo_npmrc)? && cleaned != npm_source {
            transaction.stage(&repo_npmrc, &cleaned)?;
        }
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

    if options.bun_env {
        let bun_path = repo_bun.as_ref().unwrap();
        let bun_content = transaction.contents(bun_path)?.to_owned();
        let updated_bun = bunfig::transform_bunfig_for_env(
            &bun_content,
            options.allow_all_domains,
        )?
        .ok_or_else(|| {
            AuthError::Config("No eligible registry configuration found in bunfig.toml".to_string())
        })?;
        if updated_bun != bun_content {
            transaction.stage(bun_path, &updated_bun)?;
        }

        let env_path = bun_env_path.as_ref().unwrap();
        let env_content = transaction.contents(env_path)?.to_owned();
        let updated_env = bunfig::transform_env_local(&env_content, &token)?;
        if updated_env != env_content {
            transaction.stage(env_path, &updated_env)?;
        }
    }

    let paths = transaction.commit()?;
    if paths.is_empty() {
        return Err(AuthError::Config(
            "No eligible Artifact Registry configuration found".to_string(),
        ));
    }
    Ok(RunOutcome::Updated {
        paths,
        git_status,
        bun_env_git_status,
    })
}
