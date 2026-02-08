pub use breez_sdk_spark::{DepositClaimError, Fee, SdkError, StorageError};
use flutter_rust_bridge::frb;

#[frb(mirror(DepositClaimError))]
pub enum _DepositClaimError {
    MaxDepositClaimFeeExceeded {
        tx: String,
        vout: u32,
        max_fee: Option<Fee>,
        required_fee_sats: u64,
        required_fee_rate_sat_per_vbyte: u64,
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
    InsufficientFunds,
    InvalidUuid(String),
    InvalidInput(String),
    NetworkError(String),
    StorageError(String),
    ChainServiceError(String),
    MaxDepositClaimFeeExceeded {
        tx: String,
        vout: u32,
        max_fee: Option<Fee>,
        required_fee_sats: u64,
        required_fee_rate_sat_per_vbyte: u64,
    },
    MissingUtxo {
        tx: String,
        vout: u32,
    },
    LnurlError(String),
    Signer(String),
    Generic(String),
}

#[frb(mirror(StorageError))]
pub enum _StorageError {
    Connection(String),
    Implementation(String),
    InitializationError(String),
    Serialization(String),
}
