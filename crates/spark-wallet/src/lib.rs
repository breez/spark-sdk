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
    header_provider::*,
    operator::rpc::{BalancedConnectionManager, ConnectionManager, DefaultConnectionManager},
    services::{
        CoopExitFeeQuote, CoopExitSpeedFeeQuote, CpfpUtxo, ExitSpeed, Fee,
        FreezeIssuerTokenResponse, InvoiceDescription, LightningReceivePayment,
        LightningSendPayment, LightningSendStatus, Preimage, PreimageRequestStatus,
        ReceiverTokenOutput, ServiceError, SingleUseDepositAddress, StaticDepositAddress,
        TokenInputs, TokenMintInput, TokenOutputToSpend, TokenTransaction, TokenTransactionStatus,
        TokenTransferInput, TransferId, TransferObserver, TransferObserverError, TransferStatus,
        TransferTokenOutput, TransferType, Utxo,
    },
    session_store::*,
    signer::{
        AggregateFrostRequest, ClaimLeafInput, DefaultSigner, DefaultSignerError, EncryptedSecret,
        FrostDerivation, FrostJob, FrostShareResult, FrostSigningCommitmentsWithNonces, NewLeafKey,
        OperatorPackage, OperatorRecipient, PrepareClaimRequest, PrepareLightningReceiveRequest,
        PrepareStaticDepositClaimRequest, PrepareStaticDepositRequest,
        PrepareTokenTransactionRequest, PrepareTransferRequest, PreparedClaim,
        PreparedLightningReceive, PreparedStaticDeposit, PreparedStaticDepositClaim,
        PreparedTokenTransaction, PreparedTransfer, SecretShare, SecretSource, SecretToSplit,
        SignFrostRequest, SignSparkInvoiceRequest, SignStaticDepositRefundRequest,
        SignedSparkInvoice, Signer, SignerError, SparkInvoiceKind, SparkSigner, SparkSignerAdapter,
        StartStaticDepositRefundRequest, StartedStaticDepositRefund, TokenTransactionKind,
        TransferLeafInput, VerifiableSecretShare, account_master_key, default_account_number,
        identity_master_key, identity_public_key,
    },
    ssp::*,
    token::{
        BURN_PUBLIC_KEY, GetTokenOutputsFilter, InMemoryTokenOutputStore,
        ReservationPurpose as TokenReservationPurpose, ReservationTarget, SelectionStrategy,
        TokenMetadata, TokenOutput, TokenOutputServiceError, TokenOutputStore,
        TokenOutputWithPrevOut, TokenOutputs, TokenOutputsPerStatus, TokenOutputsReservation,
        TokenOutputsReservationId, TokensConfig,
    },
    tree::{
        AutoOptimizationEvent, DEFAULT_MAX_CONCURRENT_RESERVATIONS, DEFAULT_RESERVATION_TIMEOUT,
        InMemoryTreeStore, LeafLike, LeafOptimizationOptions, Leaves, LeavesReservation,
        LeavesReservationId, OptimizationError, OptimizationOutcome, ReservationPurpose,
        ReserveResult, SelectLeavesOptions, SigningKeyshare, TargetAmounts, TreeNode, TreeNodeId,
        TreeNodeStatus, TreeServiceError, TreeStore, select_leaves_by_minimum_amount,
        select_leaves_by_target_amounts,
    },
    utils::frost::aggregate_frost,
    utils::{
        paging::{Order, PagingFilter, PagingResult},
        transactions::is_ephemeral_anchor_output,
    },
};
pub use wallet::SparkWallet;
pub use wallet_builder::WalletBuilder;

#[cfg(feature = "test-utils")]
pub use spark::session_store::tests as session_store_tests;
#[cfg(feature = "test-utils")]
pub use spark::tree::tests as tree_store_tests;

#[cfg(feature = "test-utils")]
pub use spark::token::tests as token_store_tests;
