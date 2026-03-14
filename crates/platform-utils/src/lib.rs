//! Platform utilities for cross-platform abstractions.
//!
//! This crate provides platform-agnostic utilities, including HTTP client abstractions
//! that work on both native (using bitreq) and WASM (using reqwest) platforms.

mod auth;
pub mod http;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use tokio;

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub use tokio_with_wasm::alias as tokio;

pub use auth::{
    ContentType, add_basic_auth_header, add_content_type_header, make_basic_auth_header,
};
pub use http::{DefaultHttpClient, HttpClient, HttpError, HttpResponse, create_http_client};
