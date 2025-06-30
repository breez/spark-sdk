pub(crate) mod auth;
mod connection_manager;
mod error;
mod spark_rpc_client;
pub use connection_manager::*;
pub use error::*;
pub use spark_rpc_client::*;

pub mod spark {
    tonic::include_proto!("spark");
}

pub mod spark_authn {
    tonic::include_proto!("spark_authn");
}

pub mod common {
    tonic::include_proto!("common");
}
