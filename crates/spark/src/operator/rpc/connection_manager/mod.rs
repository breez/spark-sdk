mod default;

#[cfg_attr(
    all(target_family = "wasm", target_os = "unknown"),
    path = "balanced_wasm.rs"
)]
mod balanced;

use crate::operator::OperatorConfig;
use crate::operator::rpc::transport::grpc_client::Transport;

use super::error::Result;

pub use balanced::BalancedConnectionManager;
pub use default::DefaultConnectionManager;

/// Manages gRPC connections to Spark operators.
#[macros::async_trait]
pub trait ConnectionManager: Send + Sync {
    async fn get_transport(&self, operator: &OperatorConfig) -> Result<Transport>;
}
