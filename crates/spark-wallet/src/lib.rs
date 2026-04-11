mod config;
mod error;
mod event;
mod model;
mod wallet;
mod wallet_builder;

pub use bitcoin::secp256k1::PublicKey;
pub use config::*;
pub use error::*;
pub use model::*;
pub use spark::operator::{OperatorConfig, OperatorError, OperatorPoolConfig};
pub use spark::{
    Identifier, Network,
    address::{SparkAddress, SparkAddressPaymentType},
    operator::rpc::{ConnectionManager, DefaultConnectionManager},
    services::TokensConfig,
    services::{
        BURN_PUBLIC_KEY, CoopExitFeeQuote, CoopExitSpeedFeeQuote, CpfpUtxo, DepositAddress,
        ExitSpeed, Fee, FreezeIssuerTokenResponse, InvoiceDescription, LeafOptimizationOptions,
        LightningSendPayment, LightningSendStatus, OptimizationEvent, OptimizationProgress,
        Preimage, PreimageRequestStatus, ReceiverTokenOutput, ServiceError, StaticDepositAddress,
        TokenInputs, TokenTransaction, TokenTransactionStatus, TransferId, TransferObserver,
        TransferObserverError, TransferStatus, TransferTokenOutput, TransferType, Utxo,
    },
    session_manager::*,
    signer::{
        AggregateFrostRequest, DefaultSigner, DefaultSignerError, EncryptedSecret,
        FrostSigningCommitmentsWithNonces, KeySet, KeySetType, SecretShare, SecretSource,
        SecretToSplit, SignFrostRequest, Signer, SignerError, VerifiableSecretShare,
    },
    ssp::*,
    token::{
        GetTokenOutputsFilter, InMemoryTokenOutputStore,
        ReservationPurpose as TokenReservationPurpose, ReservationTarget, SelectionStrategy,
        TokenMetadata, TokenOutput, TokenOutputServiceError, TokenOutputStore,
        TokenOutputWithPrevOut, TokenOutputs, TokenOutputsPerStatus, TokenOutputsReservation,
        TokenOutputsReservationId,
    },
    tree::{
        DEFAULT_MAX_CONCURRENT_RESERVATIONS, DEFAULT_RESERVATION_TIMEOUT, InMemoryTreeStore,
        Leaves, LeavesReservation, LeavesReservationId, ReservationPurpose, ReserveResult,
        SelectLeavesOptions, SigningKeyshare, TargetAmounts, TreeNode, TreeNodeId, TreeNodeStatus,
        TreeServiceError, TreeStore, select_leaves_by_minimum_amount,
        select_leaves_by_target_amounts,
    },
    utils::{
        paging::{Order, PagingFilter, PagingResult},
        transactions::is_ephemeral_anchor_output,
    },
};
pub use wallet::SparkWallet;
pub use wallet_builder::WalletBuilder;

#[cfg(feature = "test-utils")]
pub use spark::tree::tests as tree_store_tests;

#[cfg(feature = "test-utils")]
pub use spark::token::tests as token_store_tests;
