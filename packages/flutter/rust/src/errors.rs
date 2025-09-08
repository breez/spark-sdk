use breez_sdk_spark::Fee;
pub use breez_sdk_spark::{DepositClaimError, SdkError, StorageError};
use flutter_rust_bridge::frb;

#[frb(mirror(DepositClaimError))]
pub enum _DepositClaimError {
    DepositClaimFeeExceeded {
        tx: String,
        vout: u32,
        max_fee: Fee,
        actual_fee: u64,
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
    DepositClaimFeeExceeded {
        tx: String,
        vout: u32,
        max_fee: Fee,
        actual_fee: u64,
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
