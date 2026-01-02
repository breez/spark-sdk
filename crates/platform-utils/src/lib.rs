//! Platform utilities for cross-platform abstractions.
//!
//! This crate provides platform-agnostic utilities, including HTTP client abstractions
//! that work on both native (using bitreq) and WASM (using reqwest) platforms.

mod error;
pub mod http;

pub use error::HttpError;
pub use http::{DefaultHttpClient, HttpClient, HttpResponse, create_http_client};
