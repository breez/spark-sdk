pub(crate) mod auth;
mod connection_manager;
mod error;
mod spark_rpc_client;
pub use connection_manager::*;
pub use error::*;
pub use spark_rpc_client::*;

pub mod spark {
    #![allow(clippy::all)]
    tonic::include_proto!("spark");
}

pub mod spark_authn {
    #![allow(clippy::all)]
    tonic::include_proto!("spark_authn");
}

pub mod common {
    #![allow(clippy::all)]
    tonic::include_proto!("common");
}

pub mod spark_token {
    #![allow(clippy::all)]
    tonic::include_proto!("spark_token");
}
