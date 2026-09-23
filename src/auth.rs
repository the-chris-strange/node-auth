//! Google Cloud credential retrieval and authentication handling.
//!
//! Supports obtaining OAuth2 access tokens through multiple strategies:
//! 1. Explicitly supplied tokens (via flag or `NODE_AUTH_TOKEN` environment variable).
//! 2. Google Application Default Credentials (ADC) via service accounts or user login.
//! 3. Active `gcloud` CLI session fallback (`gcloud auth print-access-token`).

use crate::error::AuthError;
use crate::token::validate_token;
use std::process::Command;

const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

/// Retrieves an OAuth2 access token for Google Cloud Artifact Registry.
///
/// Credential resolution order:
/// 1. `explicit_token` (used unchanged if valid).
/// 2. Google Application Default Credentials (ADC) via service account file or user ADC.
/// 3. Fallback to `gcloud auth print-access-token` CLI command.
///
/// # Arguments
///
/// * `explicit_token` - Optional override token passed by the user.
/// # Errors
///
/// Returns [`AuthError::Authentication`] if credential resolution fails, or
/// [`AuthError::Config`] if an explicit token is invalid.
///
/// # Example
///
/// ```no_run
/// # async fn doc() -> Result<(), node_auth::AuthError> {
/// use node_auth::auth::get_credentials;
///
/// let token = get_credentials(Some("custom-token")).await?;
/// assert_eq!(token, "custom-token");
/// # Ok(())
/// # }
/// ```
pub async fn get_credentials(explicit_token: Option<&str>) -> Result<String, AuthError> {
    if let Some(token) = explicit_token {
        validate_token(token)?;
        return Ok(token.to_string());
    }

    // 1. Try Google Application Default Credentials (ADC)
    log::debug!("Retrieving application default credentials...");
    match get_adc_credentials().await {
        Ok(token) => {
            log::debug!("Successfully retrieved Application Default Credentials.");
            return Ok(token);
        }
        Err(err) => {
            log::debug!("Failed to retrieve ADC: {err}. Falling back to gcloud...");
        }
    }

    // 2. Fall back to gcloud CLI
    log::debug!("Retrieving credentials from gcloud CLI...");
    match get_gcloud_credentials() {
        Ok(token) => return Ok(token),
        Err(err) => {
            log::debug!("Failed to retrieve gcloud credentials: {err}");
        }
    }

    // 3. Both failed - return actionable error
    Err(AuthError::Authentication(
        "Failed to get credentials. Please run:\n\
         • `gcloud auth application-default login`\n\
         • `gcloud auth login`\n\
         or export `GOOGLE_APPLICATION_CREDENTIALS=<path/to/service/account/key.json>`"
            .to_string(),
    ))
}

async fn get_adc_credentials() -> Result<String, AuthError> {
    let provider = gcp_auth::provider().await.map_err(|e| {
        AuthError::Authentication(format!("ADC provider initialization failed: {e}"))
    })?;

    let token = provider
        .token(&[CLOUD_PLATFORM_SCOPE])
        .await
        .map_err(|e| AuthError::Authentication(format!("ADC token acquisition failed: {e}")))?;

    let token_str = token.as_str().to_string();
    validate_token(&token_str)?;
    Ok(token_str)
}

fn get_gcloud_credentials() -> Result<String, AuthError> {
    let binaries = if cfg!(windows) {
        vec!["gcloud.cmd", "gcloud.bat", "gcloud"]
    } else {
        vec!["gcloud"]
    };

    let mut last_error = String::new();

    for bin in binaries {
        log::debug!("Running `{bin} auth print-access-token`...");

        match Command::new(bin)
            .args(["auth", "print-access-token"])
            .output()
        {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8(output.stdout).map_err(|e| {
                    AuthError::Authentication(format!("gcloud returned a non-UTF-8 token: {e}"))
                })?;
                let token = stdout.trim_end_matches(['\r', '\n']);
                validate_token(token)?;
                log::debug!("Successfully retrieved credentials from gcloud CLI.");
                return Ok(token.to_string());
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                last_error = format!("`{bin}` exited with {}: {stderr}", output.status);
            }
            Err(e) => {
                last_error = format!("Failed to spawn `{bin}`: {e}");
            }
        }
    }

    Err(AuthError::Authentication(last_error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_explicit_token() {
        let token = get_credentials(Some("my-explicit-token")).await.unwrap();
        assert_eq!(token, "my-explicit-token");
    }

    #[tokio::test]
    async fn test_explicit_token_rejects_unsafe_characters() {
        assert!(
            get_credentials(Some("  my-token-with-spaces  \n"))
                .await
                .is_err()
        );
    }
}
