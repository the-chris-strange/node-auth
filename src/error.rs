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

    /// YAML serialization or deserialization failure for Yarn configs.
    #[error("YAML parse/format error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    /// Regular expression compilation or matching error.
    #[error("Regex error: {0}")]
    Regex(#[from] regex::Error),
}