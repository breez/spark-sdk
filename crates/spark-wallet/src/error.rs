use spark::{operator::rpc::OperatorRpcError, services::ServiceError};
use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum SparkWalletError {
    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Insufficient funds")]
    InsufficientFunds,

    #[error("Invalid network")]
    InvalidNetwork,

    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    #[error("Invalid output index")]
    InvalidOutputIndex,

    #[error("Self payment not allowed")]
    SelfPaymentNotAllowed,

    #[error("Leaves not found")]
    LeavesNotFound,

    #[error("Not a deposit output")]
    NotADepositOutput,

    #[error("Signer error: {0}")]
    SignerServiceError(#[from] spark::signer::SignerError),

    #[error("Deposit address used")]
    DepositAddressUsed,

    #[error("Operator RPC error: {0}")]
    OperatorRpcError(Box<spark::operator::rpc::OperatorRpcError>),

    #[error("Operator pool error: {0}")]
    OperatorPoolError(String),

    #[error("Address error: {0}")]
    AddressError(#[from] spark::address::error::AddressError),

    #[error("Tree service error: {0}")]
    TreeServiceError(#[from] spark::tree::TreeServiceError),

    #[error("Token output service error: {0}")]
    TokenOutputServiceError(#[from] spark::token::TokenOutputServiceError),

    #[error("Service error: {0}")]
    ServiceError(#[from] spark::services::ServiceError),

    #[error("SSP error: {0}")]
    SspError(#[from] spark::ssp::ServiceProviderError),

    #[error("Generic error: {0}")]
    Generic(String),
}

impl From<spark::operator::rpc::OperatorRpcError> for SparkWalletError {
    fn from(error: spark::operator::rpc::OperatorRpcError) -> Self {
        SparkWalletError::OperatorRpcError(Box::new(error))
    }
}

impl SparkWalletError {
    pub fn is_operator_unavailable(&self) -> bool {
        let rpc_error = match self {
            SparkWalletError::OperatorRpcError(error) => error.as_ref(),
            SparkWalletError::ServiceError(ServiceError::ServiceConnectionError(error)) => {
                error.as_ref()
            }
            SparkWalletError::ServiceError(ServiceError::RequestError(status)) => {
                return is_temporary(status.code());
            }
            _ => return false,
        };
        match rpc_error {
            OperatorRpcError::Transport(_) => true,
            OperatorRpcError::Connection(status) => is_temporary(status.code()),
            _ => false,
        }
    }
}

fn is_temporary(code: tonic::Code) -> bool {
    matches!(
        code,
        tonic::Code::Unavailable
            | tonic::Code::DeadlineExceeded
            | tonic::Code::ResourceExhausted
            | tonic::Code::Aborted
    )
}

#[cfg(test)]
mod tests {
    use tonic::{Code, Status};

    use super::*;

    fn operator_status(code: Code) -> SparkWalletError {
        SparkWalletError::ServiceError(ServiceError::ServiceConnectionError(Box::new(
            OperatorRpcError::Connection(Box::new(Status::new(code, "operator"))),
        )))
    }

    #[test]
    fn unreachable_or_busy_operators_are_unavailable() {
        for code in [
            Code::Unavailable,
            Code::DeadlineExceeded,
            Code::ResourceExhausted,
            Code::Aborted,
        ] {
            assert!(operator_status(code).is_operator_unavailable(), "{code:?}");
        }
        let transport = SparkWalletError::from(OperatorRpcError::Transport("refused".to_string()));
        assert!(transport.is_operator_unavailable());
    }

    #[test]
    fn a_refusal_is_not_unavailability() {
        for code in [
            Code::FailedPrecondition,
            Code::InvalidArgument,
            Code::NotFound,
            Code::PermissionDenied,
            Code::Internal,
        ] {
            assert!(!operator_status(code).is_operator_unavailable(), "{code:?}");
        }
        assert!(
            !SparkWalletError::ValidationError("invalid".to_string()).is_operator_unavailable()
        );
    }
}
