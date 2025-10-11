pub mod client;
pub mod model;
pub mod signer;
pub mod signing_client;

#[allow(clippy::doc_markdown)]
pub mod proto {
    tonic::include_proto!("sync");
}
