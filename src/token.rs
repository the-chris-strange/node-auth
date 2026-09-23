//! Access-token validation.

use crate::AuthError;

/// Reject tokens that cannot safely be represented in a quoted `.npmrc` value.
pub fn validate_token(token: &str) -> Result<(), AuthError> {
    if token.is_empty()
        || token
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '"' || c == '\\')
    {
        return Err(AuthError::Config(
            "Access token is empty or contains whitespace, control, quote, or backslash characters"
                .to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_safe_token() {
        assert!(validate_token("safe-token_123.abc").is_ok());
    }

    #[test]
    fn rejects_empty_or_unsafe_tokens() {
        for token in ["", "has space", "has\nnewline", "has\"quote", "has\\slash"] {
            assert!(validate_token(token).is_err(), "accepted {token:?}");
        }
    }
}
