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
//!         verbose: true,
//!         ..Default::default()
//!     };
//!     run(&options).await
//! }
//! ```

#![warn(missing_docs)]

pub mod auth;
pub mod cli;
pub mod error;
pub mod logger;
pub mod npmrc;
pub mod vcs;
pub mod yarnrc;

use std::path::{Path, PathBuf};
use colored::Colorize;
pub use error::AuthError;

/// Configuration options for configuring Node authentication.
#[derive(Debug, Clone, Default)]
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

    /// Print verbose/debug output during execution.
    pub verbose: bool,

    /// Explicitly enable (`Some(true)`) or disable (`Some(false)`) updating `.yarnrc.yml`.
    /// When `None`, Yarn updating is auto-detected based on `.yarnrc.yml` presence.
    pub yarn: Option<bool>,

    /// Only retrieve and print the access token to standard output without modifying files.
    pub print_token: bool,
}

/// Executes authentication and updates `.npmrc` and `.yarnrc.yml` configuration files.
///
/// # Arguments
///
/// * `options` - Parameters controlling file paths, tokens, domain restrictions, and verbosity.
///
/// # Errors
///
/// Returns [`AuthError`] if authentication fails, files cannot be read/written,
/// or no Artifact Registry configuration is discovered.
pub async fn run(options: &Options) -> Result<(), AuthError> {
    logger::init(options.verbose, !options.print_token);

    if options.print_token {
        let token = auth::get_credentials(options.token.as_deref(), options.verbose).await?;
        println!("{token}");
        return Ok(());
    }

    let home_dir = dirs::home_dir().ok_or_else(|| {
        AuthError::Config("Unable to determine user home directory".to_string())
    })?;

    // Determine npmrc paths
    let (repo_npmrc, cred_npmrc) = if options.local_credential {
        let cwd = std::env::current_dir()?;
        // Check gitignore and emit warnings if appropriate
        vcs::check_local_credential_safety(&cwd);

        let local_npmrc = cwd.join(".npmrc");

        let repo = if let Some(ref r) = options.repo_config {
            r.clone()
        } else if local_npmrc.exists() {
            local_npmrc.clone()
        } else {
            home_dir.join(".npmrc")
        };

        (repo, local_npmrc)
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

        (repo, cred)
    };

    log::debug!("Using repo config (.npmrc): {}", repo_npmrc.display());
    log::debug!("Using credential config (.npmrc): {}", cred_npmrc.display());

    // Retrieve token
    let token = auth::get_credentials(options.token.as_deref(), options.verbose).await?;

    // Update npmrc configuration
    npmrc::update_npmrc_configs(
        &repo_npmrc,
        &cred_npmrc,
        &token,
        options.allow_all_domains,
        options.verbose,
    )?;

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

    if should_update_yarn {
        log::debug!("Using yarn repo config: {}", repo_yarn.display());
        log::debug!("Using yarn credential config: {}", cred_yarn.display());
        yarnrc::update_yarn_configs(&repo_yarn, &cred_yarn, &token, options.verbose)?;
    }

    log::info!("Successfully configured Artifact Registry credentials.");
    println!("{}", "Success!".green().bold());

    Ok(())
}