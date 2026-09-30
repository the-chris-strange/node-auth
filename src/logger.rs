//! CLI-only logging and presentation.

use crate::vcs::GitStatus;
use crate::{AuthError, RunOutcome};
use colored::Colorize;
use log::{Level, LevelFilter, Log, Metadata, Record};

struct CliLogger;
static LOGGER: CliLogger = CliLogger;

impl Log for CliLogger {
  fn enabled(&self, metadata: &Metadata) -> bool {
    metadata.level() <= log::max_level()
  }

  fn log(&self, record: &Record) {
    if self.enabled(record.metadata()) {
      eprintln!("{} {}", level_label(record.level()), record.args());
    }
  }

  fn flush(&self) {}
}

fn level_label(level: Level) -> String {
  match level {
    Level::Error => "[ERROR]".red().bold().to_string(),
    Level::Warn => "[WARN]".yellow().bold().to_string(),
    Level::Info => "[INFO]".green().bold().to_string(),
    Level::Debug => "[DEBUG]".cyan().bold().to_string(),
    Level::Trace => "[TRACE]".magenta().bold().to_string(),
  }
}

/// Initialize logging for a CLI invocation. All diagnostic output goes to stderr.
pub fn init(verbose: bool) {
  let _ = log::set_logger(&LOGGER);
  log::set_max_level(if verbose {
    LevelFilter::Debug
  } else {
    LevelFilter::Warn
  });
}

/// Present a successful library result to a CLI user.
pub fn present_outcome(outcome: RunOutcome) {
  match outcome {
    RunOutcome::Token(token) => println!("{token}"),
    RunOutcome::Updated {
      paths,
      git_status,
      bun_env_git_status,
    } => {
      if let Some(status) = git_status {
        match status {
          GitStatus::GitRepoNotIgnored => eprintln!(
            "{} Local .npmrc is tracked or not ignored by Git; it may expose credentials if committed.",
            "Warning:".yellow().bold()
          ),
          GitStatus::NotGitRepo => eprintln!(
            "{} Local .npmrc is outside a Git repository; keep credentials out of version control.",
            "Warning:".yellow().bold()
          ),
          GitStatus::Unavailable => eprintln!(
            "{} Could not verify Git ignore status for local .npmrc.",
            "Warning:".yellow().bold()
          ),
          GitStatus::GitRepoIgnored => {}
        }
      }
      if let Some(status) = bun_env_git_status {
        match status {
          GitStatus::GitRepoNotIgnored => eprintln!(
            "{} .env.local is tracked or not ignored by Git; it may expose credentials if committed.",
            "Warning:".yellow().bold()
          ),
          GitStatus::NotGitRepo => eprintln!(
            "{} .env.local is outside a Git repository; keep credentials out of version control.",
            "Warning:".yellow().bold()
          ),
          GitStatus::Unavailable => eprintln!(
            "{} Could not verify Git ignore status for .env.local.",
            "Warning:".yellow().bold()
          ),
          GitStatus::GitRepoIgnored => {}
        }
      }
      if !paths.is_empty() {
        println!(
          "{} Updated {} configuration file(s).",
          "Success!".green().bold(),
          paths.len()
        );
      }
    }
  }
}

/// Present one CLI error on stderr.
pub fn present_error(error: &AuthError) {
  eprintln!("{} {}", "Error:".red().bold(), format_error(error));
}

// Keep remediation and command-line option names in the CLI presentation layer.
fn format_error(error: &AuthError) -> String {
  match error {
    AuthError::NoNpmRegistry => format!(
      "{error}.\n\
       Ensure your .npmrc contains a registry line such as:\n\
       @my-scope:registry=https://<region>-npm.pkg.dev/<project>/<repo>/\n\
       Run `gcloud artifacts print-settings npm` to generate configuration."
    ),
    AuthError::CredentialsUnavailable => format!(
      "{error}.\n\
       Please run:\n\
       • `gcloud auth application-default login`\n\
       • `gcloud auth login`\n\
       or export `GOOGLE_APPLICATION_CREDENTIALS=<path/to/service/account/key.json>`"
    ),
    AuthError::BunEnvLocalCredentialConflict => {
      "Configuration error: --bun-env cannot be used with --local-credential".to_string()
    }
    AuthError::BunEnvDisabledConflict => {
      "Configuration error: --bun-env cannot be used with --bun false".to_string()
    }
    _ => error.to_string(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn npm_guidance_has_explicit_line_layout() {
    assert_eq!(
      format_error(&AuthError::NoNpmRegistry),
      concat!(
        "Configuration error: No eligible registry configuration found in .npmrc.\n",
        "Ensure your .npmrc contains a registry line such as:\n",
        "@my-scope:registry=https://<region>-npm.pkg.dev/<project>/<repo>/\n",
        "Run `gcloud artifacts print-settings npm` to generate configuration."
      )
    );
  }

  #[test]
  fn authentication_guidance_has_explicit_line_layout() {
    assert_eq!(
      format_error(&AuthError::CredentialsUnavailable),
      concat!(
        "Authentication error: Failed to get credentials from ADC or gcloud.\n",
        "Please run:\n",
        "• `gcloud auth application-default login`\n",
        "• `gcloud auth login`\n",
        "or export `GOOGLE_APPLICATION_CREDENTIALS=<path/to/service/account/key.json>`"
      )
    );
  }

  #[test]
  fn option_conflicts_use_cli_flag_names() {
    assert_eq!(
      format_error(&AuthError::BunEnvLocalCredentialConflict),
      "Configuration error: --bun-env cannot be used with --local-credential"
    );
    assert_eq!(
      format_error(&AuthError::BunEnvDisabledConflict),
      "Configuration error: --bun-env cannot be used with --bun false"
    );
  }

  #[test]
  fn ordinary_errors_keep_their_diagnostic() {
    let error = AuthError::Config("Invalid registry URL".to_string());
    assert_eq!(format_error(&error), error.to_string());
  }
}
