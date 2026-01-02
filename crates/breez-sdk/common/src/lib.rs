pub mod breez_server;
pub mod buy;
pub mod dns;
pub mod error;
pub mod fiat;
pub mod grpc;
pub mod input;
pub mod invoice;
pub mod lnurl;
pub mod network;
pub mod sync;
pub mod tonic_wrap;
pub mod utils;

// Re-export platform-utils HTTP types for backwards compatibility
pub use platform_utils::{
    DefaultHttpClient, HttpClient, HttpError, HttpResponse, create_http_client,
};

// Re-export HttpError as ServiceConnectivityError for backwards compatibility
pub use platform_utils::HttpError as ServiceConnectivityError;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;
