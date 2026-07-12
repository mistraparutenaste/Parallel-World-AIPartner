//! Endpoint validation: local servers must stay on loopback.

use url::Url;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EndpointError {
    #[error("invalid base url: {0}")]
    Invalid(String),
    #[error("non-loopback endpoints require allow_remote: {0}")]
    RemoteNotAllowed(String),
}

/// Parses and validates the base URL. Non-loopback hosts are only
/// accepted when `allow_remote` is set (設計spec 5章).
///
/// # Errors
///
/// Returns [`EndpointError`] for unparsable URLs, non-HTTP schemes,
/// or remote hosts without `allow_remote`.
pub fn validate_base_url(base_url: &str, allow_remote: bool) -> Result<Url, EndpointError> {
    let url = Url::parse(base_url).map_err(|error| EndpointError::Invalid(error.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(EndpointError::Invalid(format!(
            "unsupported scheme: {}",
            url.scheme()
        )));
    }
    let is_loopback = match url.host() {
        Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    if !is_loopback && !allow_remote {
        return Err(EndpointError::RemoteNotAllowed(base_url.to_owned()));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::{EndpointError, validate_base_url};

    #[test]
    fn accepts_loopback_hosts() {
        for url in [
            "http://127.0.0.1:8080/v1",
            "http://localhost:8080/v1",
            "http://[::1]:8080/v1",
        ] {
            assert!(validate_base_url(url, false).is_ok(), "{url}");
        }
    }

    #[test]
    fn rejects_remote_hosts_without_allow_remote() {
        let error = validate_base_url("https://api.example.com/v1", false).unwrap_err();
        assert!(matches!(error, EndpointError::RemoteNotAllowed(_)));
    }

    #[test]
    fn accepts_remote_hosts_when_explicitly_allowed() {
        assert!(validate_base_url("https://api.example.com/v1", true).is_ok());
    }

    #[test]
    fn rejects_non_http_schemes() {
        assert!(matches!(
            validate_base_url("file:///etc/passwd", true).unwrap_err(),
            EndpointError::Invalid(_)
        ));
    }
}
