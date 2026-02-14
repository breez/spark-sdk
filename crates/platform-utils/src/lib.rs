//! Platform utilities for cross-platform abstractions.
//!
//! This crate provides platform-agnostic utilities, including HTTP client abstractions
//! that work on both native (using bitreq) and WASM (using reqwest) platforms.

mod auth;
mod error;
pub mod http;

pub use auth::{
    ContentType, add_basic_auth_header, add_content_type_header, make_basic_auth_header,
};
pub use error::HttpError;
pub use http::{DefaultHttpClient, HttpClient, HttpResponse, create_http_client};
