//! Command-line argument parsing and CLI definitions for `node-auth`.

use std::path::PathBuf;
use clap::Parser;
use crate::Options;

/// Command-line arguments for the `node-auth` CLI.
#[derive(Parser, Debug, Clone, PartialEq, Eq)]
#[command(
    name = "node-auth",
    about = "Authenticates Node (npm, yarn, pnpm) to Google Artifact Registry using Google Cloud ADC",
    version
)]
pub struct Cli {
    /// Path to the .npmrc file to read registry configs from.
    /// Defaults to project-level .npmrc if present, otherwise user-level ~/.npmrc.
    #[arg(long, env = "NODE_AUTH_REPO_CONFIG")]
    pub repo_config: Option<PathBuf>,

    /// Path to the .npmrc file to write credentials to.
    /// Defaults to user-level ~/.npmrc.
    #[arg(long, env = "NODE_AUTH_CREDENTIAL_CONFIG")]
    pub credential_config: Option<PathBuf>,

    /// Forces writing credentials to the current directory's .npmrc (creating it if missing).
    /// Emits a warning if running in a Git repository without .npmrc in .gitignore.
    #[arg(short = 'l', long)]
    pub local_credential: bool,

    /// Path to the .yarnrc.yml file to read registry configs from.
    /// Defaults to project-level .yarnrc.yml if present, otherwise user-level ~/.yarnrc.yml.
    #[arg(long)]
    pub repo_config_yarn: Option<PathBuf>,

    /// Path to the .yarnrc.yml file to write credentials to.
    /// Defaults to user-level ~/.yarnrc.yml.
    #[arg(long)]
    pub credential_config_yarn: Option<PathBuf>,

    /// Explicitly enable or disable updating Yarn configuration (.yarnrc.yml).
    /// By default, Yarn is updated if .yarnrc.yml exists or is explicitly specified.
    #[arg(long)]
    pub yarn: Option<bool>,

    /// Explicit token to use instead of querying ADC or gcloud.
    #[arg(long, env = "NODE_AUTH_TOKEN")]
    pub token: Option<String>,

    /// Allow all registry domains to attach the auth token to (not only *-npm.pkg.dev).
    #[arg(long)]
    pub allow_all_domains: bool,

    /// Print verbose output during execution.
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,

    /// Only retrieve and print the access token to stdout without modifying files.
    #[arg(long)]
    pub print_token: bool,
}

impl Cli {
    /// Converts the parsed CLI arguments into library execution [`Options`].
    pub fn to_options(&self) -> Options {
        Options {
            repo_config: self.repo_config.clone(),
            credential_config: self.credential_config.clone(),
            repo_config_yarn: self.repo_config_yarn.clone(),
            credential_config_yarn: self.credential_config_yarn.clone(),
            local_credential: self.local_credential,
            token: self.token.clone(),
            allow_all_domains: self.allow_all_domains,
            verbose: self.verbose,
            yarn: self.yarn,
            print_token: self.print_token,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_verbose_flags() {
        let cli_short = Cli::try_parse_from(["node-auth", "-v"]).unwrap();
        assert!(cli_short.verbose);

        let cli_long = Cli::try_parse_from(["node-auth", "--verbose"]).unwrap();
        assert!(cli_long.verbose);

        let cli_default = Cli::try_parse_from(["node-auth"]).unwrap();
        assert!(!cli_default.verbose);
    }

    #[test]
    fn test_cli_local_credential_flags() {
        let cli_short = Cli::try_parse_from(["node-auth", "-l"]).unwrap();
        assert!(cli_short.local_credential);

        let cli_long = Cli::try_parse_from(["node-auth", "--local-credential"]).unwrap();
        assert!(cli_long.local_credential);
    }

    #[test]
    fn test_cli_print_token_flag() {
        let cli = Cli::try_parse_from(["node-auth", "--print-token", "--token", "custom-token"]).unwrap();
        assert!(cli.print_token);
        assert_eq!(cli.token.as_deref(), Some("custom-token"));
    }

    #[test]
    fn test_cli_to_options() {
        let cli = Cli::try_parse_from(["node-auth", "-v", "-l", "--token", "tok"]).unwrap();
        let options = cli.to_options();
        assert!(options.verbose);
        assert!(options.local_credential);
        assert_eq!(options.token.as_deref(), Some("tok"));
    }
}