#[derive(Debug, thiserror::Error)]
pub enum TransferServiceError {
    #[error("Failed to extend time lock: {0}")]
    ExtendTimeLockError(String),
}
