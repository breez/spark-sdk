//! Authentication utilities for HTTP clients.

use base64::Engine;

/// Create a Basic authentication header value.
///
/// Returns a string in the format `Basic <base64-encoded-credentials>`.
pub fn make_basic_auth_header(username: &str, password: &str) -> String {
    let credentials = format!("{username}:{password}");
    let encoded = base64::engine::general_purpose::STANDARD.encode(credentials.as_bytes());
    format!("Basic {encoded}")
}
