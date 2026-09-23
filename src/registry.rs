//! Shared URL validation for npm and Yarn registries.

use crate::AuthError;
use url::Url;

/// Policy governing which registry hosts may receive the access token.
#[derive(Clone, Copy, Debug)]
pub struct RegistryPolicy {
    /// Permit registries outside Google Artifact Registry when explicitly requested.
    pub allow_all_domains: bool,
}

/// A validated HTTPS registry URL and its npm credential key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryUrl {
    url: Url,
    npm_key: String,
}

impl RegistryUrl {
    /// The normalized URL, suitable for a Yarn `npmRegistryServer` value.
    pub fn as_url(&self) -> &str {
        self.url.as_str()
    }

    /// The normalized `//host/path/` key used by npm credentials.
    pub fn npm_key(&self) -> &str {
        &self.npm_key
    }
}

impl RegistryPolicy {
    /// Parse a registry URL. Untrusted hosts return `Ok(None)`; malformed URLs return an error.
    pub fn parse(&self, input: &str) -> Result<Option<RegistryUrl>, AuthError> {
        let mut url = Url::parse(input)
            .map_err(|e| AuthError::Config(format!("Invalid registry URL: {e}")))?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(AuthError::Config(
                "Registry URL must be HTTPS without credentials, query, or fragment".to_string(),
            ));
        }

        let host = url.host_str().unwrap().to_owned();
        let is_gar = host.strip_suffix("-npm.pkg.dev").is_some_and(|region| {
            !region.is_empty()
                && region
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
        if !self.allow_all_domains && !is_gar {
            return Ok(None);
        }
        if is_gar && url.port().is_some() && !self.allow_all_domains {
            return Err(AuthError::Config(
                "Artifact Registry URL may not use a nonstandard port".to_string(),
            ));
        }
        if is_gar
            && url
                .path_segments()
                .is_none_or(|segments| segments.filter(|s| !s.is_empty()).count() < 2)
        {
            return Err(AuthError::Config(
                "Artifact Registry URL must include project and repository".to_string(),
            ));
        }

        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }
        let authority = match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host,
        };
        let npm_key = format!("//{authority}{}", url.path());
        Ok(Some(RegistryUrl { url, npm_key }))
    }

    /// Parse an npm credential key (`//host/path/`) using the same policy.
    pub fn parse_npm_key(&self, key: &str) -> Result<Option<RegistryUrl>, AuthError> {
        if !key.starts_with("//") {
            return Err(AuthError::Config("Invalid npm registry key".to_string()));
        }
        self.parse(&format!("https:{key}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_checks_parsed_host_and_scheme() {
        let policy = RegistryPolicy {
            allow_all_domains: false,
        };
        assert_eq!(
            policy.parse("https://registry.example.com/p/r").unwrap(),
            None
        );
        assert_eq!(
            policy
                .parse("https://us-npm.pkg.dev.evil.test/p/r")
                .unwrap(),
            None
        );
        assert!(policy.parse("http://us-npm.pkg.dev/p/r").is_err());
        assert!(policy.parse("https://user@us-npm.pkg.dev/p/r").is_err());
        assert!(policy.parse("https://us-npm.pkg.dev/p/r?x=1").is_err());
        assert!(policy.parse("https://us-npm.pkg.dev:444/p/r").is_err());
        assert_eq!(
            policy
                .parse("https://us-npm.pkg.dev/p/r")
                .unwrap()
                .unwrap()
                .npm_key(),
            "//us-npm.pkg.dev/p/r/"
        );
    }

    #[test]
    fn override_allows_other_https_hosts() {
        let policy = RegistryPolicy {
            allow_all_domains: true,
        };
        assert!(
            policy
                .parse("https://registry.example.com/private")
                .unwrap()
                .is_some()
        );
    }
}
