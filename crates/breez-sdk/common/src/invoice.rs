use crate::{input::Bolt11InvoiceDetails, network::BitcoinNetwork};

pub type InvoiceResult<T, E = InvoiceError> = Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum InvoiceError {
    #[error("{0}")]
    General(String),

    #[error("{0}")]
    InvalidNetwork(String),

    #[error("{0}")]
    Validation(String),
}

impl InvoiceError {
    pub fn general(err: &str) -> Self {
        Self::General(err.to_string())
    }

    pub fn invalid_network(err: &str) -> Self {
        Self::InvalidNetwork(err.to_string())
    }

    pub fn validation(err: &str) -> Self {
        Self::Validation(err.to_string())
    }
}

// Validate that the LNInvoice network matches the provided network
pub fn validate_network(
    invoice: &Bolt11InvoiceDetails,
    network: BitcoinNetwork,
) -> InvoiceResult<()> {
    if invoice.network != network {
        return Err(InvoiceError::invalid_network(
            "Invoice network does not match config",
        ));
    }

    Ok(())
}
