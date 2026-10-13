//! Platform utilities for cross-platform abstractions.
//!
//! This crate provides platform-agnostic utilities, including HTTP client abstractions
//! built on reqwest that work on both native and WASM platforms.

// The spawn and reqwest rules in the root clippy.toml guard shipped code, not tests.
#![cfg_attr(test, allow(clippy::disallowed_methods))]

mod auth;
pub mod http;
pub mod proxy;
mod spawn;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use tokio;

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub use tokio_with_wasm::alias as tokio;

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub use web_time as time;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use std::time;

pub use auth::{
    ContentType, add_basic_auth_header, add_content_type_header, make_basic_auth_header,
};
pub use http::{
    DefaultHttpClient, HttpClient, HttpError, HttpResponse, MAX_RESPONSE_BYTES, REQUEST_TIMEOUT,
    RedirectFilter, create_http_client, create_http_client_with_proxy,
    create_http_client_with_redirect_filter, read_capped_bytes, read_capped_text,
};
pub use proxy::ProxyConfig;
pub use spawn::spawn;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use proxy::Socks5Connector;
