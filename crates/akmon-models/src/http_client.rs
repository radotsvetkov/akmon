//! Shared HTTP client builders with explicit timeouts.

use std::time::Duration;

/// Builds an HTTP client with connect and overall request timeouts.
///
/// Returns an error when the underlying TLS/runtime stack cannot construct a client.
pub fn build_http_client(connect_secs: u64, request_secs: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(connect_secs))
        .timeout(Duration::from_secs(request_secs))
        .build()
        .map_err(|e| format!("HTTP client builder failed: {e}"))
}
