//! Error type shared by all adapter ports.

/// Failure inside an adapter. Messages must not contain secrets.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct PortError(pub String);
