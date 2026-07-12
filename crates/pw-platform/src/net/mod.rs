//! Network endpoint policy shared by the local-server adapters.

mod endpoint;

pub use endpoint::{EndpointError, validate_base_url};
