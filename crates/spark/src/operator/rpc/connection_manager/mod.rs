mod default;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
mod balanced;

use crate::operator::OperatorConfig;
use crate::operator::rpc::transport::grpc_client::Transport;

use super::error::Result;

pub use default::DefaultConnectionManager;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use balanced::BalancedConnectionManager;

/// Manages gRPC connections to Spark operators.
#[macros::async_trait]
pub trait ConnectionManager: Send + Sync {
    async fn get_transport(&self, operator: &OperatorConfig) -> Result<Transport>;
}
