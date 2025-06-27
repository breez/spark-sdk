use crate::signer::SignerError;

#[derive(Debug, thiserror::Error)]
pub enum TransferServiceError {
    #[error("Failed to extend time lock: {0}")]
    ExtendTimeLockError(String),
    #[error("Signer error: {0}")]
    SignerError(#[from] SignerError),
    #[error("Transfer verification failed: {0}")]
    TransferVerificationError(String),
    #[error("Max retries exceeded")]
    MaxRetriesExceeded,
    #[error("No leaves to claim")]
    NoLeavesToClaim,
    #[error("Claim transfer failed: {0}")]
    ClaimTransferError(String),
    #[error("Generic error: {0}")]
    Generic(String),
}
