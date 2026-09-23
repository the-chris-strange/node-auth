//! Error types for authentication and configuration failures.

use thiserror::Error;

/// Errors that can occur during authentication or configuration file manipulation.
#[derive(Error, Debug)]
pub enum AuthError {
    /// Authentication failure when resolving credentials via ADC or gcloud CLI.
    #[error("Authentication error:\n{0}")]
    Authentication(String),

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

    /// Regular expression compilation or matching error.
    #[error("Regex error: {0}")]
    Regex(#[from] regex::Error),
}
