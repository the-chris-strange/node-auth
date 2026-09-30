//! Error types for authentication and configuration failures.

use thiserror::Error;

/// Errors that can occur during authentication or configuration file manipulation.
#[derive(Error, Debug)]
pub enum AuthError {
  /// Authentication failure when resolving credentials via ADC or gcloud CLI.
  #[error("Authentication error: {0}")]
  Authentication(String),

  /// No eligible registry was discovered in the npm configuration.
  #[error("Configuration error: No eligible registry configuration found in .npmrc")]
  NoNpmRegistry,

  /// Neither Application Default Credentials nor gcloud supplied credentials.
  #[error("Authentication error: Failed to get credentials from ADC or gcloud")]
  CredentialsUnavailable,

  /// Bun environment credentials conflict with local npm credentials.
  #[error(
    "Configuration error: Bun environment credentials cannot be combined with local npm credentials"
  )]
  BunEnvLocalCredentialConflict,

  /// Bun environment credentials were requested while Bun support was disabled.
  #[error("Configuration error: Bun environment credentials require Bun support")]
  BunEnvDisabledConflict,

  /// File system or I/O failure.
  #[error("IO error: {0}")]
  Io(#[from] std::io::Error),

  /// Missing or invalid configuration (e.g. no Artifact Registry scoped registry found).
  #[error("Configuration error: {0}")]
  Config(String),

  /// Invalid source or destination Yarn configuration.
  #[error("YAML parse error in {input}: {source}")]
  YamlInput {
    /// Whether the error came from the source or credential configuration.
    input: &'static str,
    /// The underlying YAML parse error.
    #[source]
    source: yaml_edit::YamlError,
  },

  /// Invalid Bun configuration.
  #[error("TOML parse error in bunfig.toml: {0}")]
  TomlInput(#[from] toml_edit::TomlError),

  /// Regular expression compilation or matching error.
  #[error("Regex error: {0}")]
  Regex(#[from] regex::Error),
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn library_errors_do_not_include_cli_guidance() {
    for error in [
      AuthError::NoNpmRegistry,
      AuthError::CredentialsUnavailable,
      AuthError::BunEnvLocalCredentialConflict,
      AuthError::BunEnvDisabledConflict,
      AuthError::Authentication("provider failed".to_string()),
    ] {
      let message = error.to_string();
      assert!(!message.contains('\n'));
      assert!(!message.contains("--"));
      assert!(!message.contains("Please run"));
      assert!(!message.contains("Ensure"));
    }
  }
}
