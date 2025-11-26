use breez_sdk_spark::Fee;
pub use breez_sdk_spark::{DepositClaimError, SdkError, StorageError};
use flutter_rust_bridge::frb;

#[frb(mirror(DepositClaimError))]
pub enum _DepositClaimError {
    MaxDepositClaimFeeExceeded {
        tx: String,
        vout: u32,
        max_fee: Option<u64>,
        required_fee: u64,
    },
    MissingUtxo {
        tx: String,
        vout: u32,
    },
    Generic {
        message: String,
    },
}

#[frb(mirror(SdkError))]
pub enum _SdkError {
    SparkError(String),
    InvalidUuid(String),
    InvalidInput(String),
    NetworkError(String),
    StorageError(String),
    ChainServiceError(String),
    MaxDepositClaimFeeExceeded {
        tx: String,
        vout: u32,
        max_fee: Option<u64>,
        required_fee: u64,
    },
    MissingUtxo {
        tx: String,
        vout: u32,
    },
    LnurlError(String),
    Generic(String),
}

#[frb(mirror(StorageError))]
pub enum _StorageError {
    Implementation(String),
    InitializationError(String),
    Serialization(String),
}
