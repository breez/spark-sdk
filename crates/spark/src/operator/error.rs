use thiserror::Error;

#[derive(Debug, Copy, Clone, Error)]
pub enum OperatorError {
    #[error("Invalid coordinator index")]
    InvalidCoordinatorIndex,
    #[error("Invalid operator id")]
    InvalidOperatorId,
}
