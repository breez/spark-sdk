mod config;
mod error;
mod event;
mod model;
mod unilateral_exit;
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
        CoopExitFeeQuote, CoopExitSpeedFeeQuote, CpfpChild, CpfpInput, ExitSpeed, Fee,
        FreezeIssuerTokenResponse, InvoiceDescription, LightningReceivePayment,
        LightningSendPayment, LightningSendStatus, Preimage, PreimageRequestStatus,
        ReceiverTokenOutput, ServiceError, SingleUseDepositAddress, StaticDepositAddress,
        TokenInputs, TokenMintInput, TokenOutputToSpend, TokenTransaction, TokenTransactionStatus,
        TokenTransferInput, TransferId, TransferObserver, TransferObserverError, TransferStatus,
        TransferTokenOutput, TransferType, UnilateralExitPlan, UnilateralExitSelectedLeaf, Utxo,
        build_cpfp_child, build_unilateral_exit_chain, compute_sweep_fee, csv_timelock,
        p2tr_key_path_input_weight, p2wpkh_input_weight, walk_unilateral_exit_chain,
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
        BURN_PUBLIC_KEY, GetTokenOutputsFilter, InMemoryTokenOutputStore, MAX_TOKEN_TX_OUTPUTS,
        PreparedTokenPackage, PreparedTokenReceiverOutput, PreparedTokenTransfer,
        ReservationPurpose as TokenReservationPurpose, ReservationTarget, SelectionStrategy,
        TokenMetadata, TokenOutpoint, TokenOutput, TokenOutputServiceError, TokenOutputStore,
        TokenOutputWithPrevOut, TokenOutputs, TokenOutputsPerStatus, TokenOutputsReservation,
        TokenOutputsReservationId, TokensConfig, select_token_outputs_from,
    },
    tree::{
        AutoOptimizationEvent, DEFAULT_MAX_CONCURRENT_RESERVATIONS, DEFAULT_RESERVATION_TIMEOUT,
        InMemoryTreeStore, LeafLike, LeafOptimizationOptions, LeafSelection, Leaves,
        LeavesReservation, LeavesReservationId, OptimizationError, OptimizationOutcome,
        ReservationPurpose, ReserveResult, SelectLeavesOptions, SigningKeyshare, TargetAmounts,
        TreeNode, TreeNodeId, TreeNodeStatus, TreeServiceError, TreeStore, VerifiedLeafKeys,
        select_leaves_by_minimum_amount, select_leaves_by_target_amounts,
        verified_leaf_keys_from_leaves,
    },
    utils::frost::aggregate_frost,
    utils::{
        paging::{Order, PagingFilter, PagingResult},
        transactions::is_ephemeral_anchor_output,
    },
};
pub use unilateral_exit::*;
pub use wallet::{SendPackagePreparation, SparkWallet};
pub use wallet_builder::WalletBuilder;

#[cfg(feature = "test-utils")]
pub use spark::session_store::tests as session_store_tests;
#[cfg(feature = "test-utils")]
pub use spark::tree::tests as tree_store_tests;

#[cfg(feature = "test-utils")]
pub use spark::token::tests as token_store_tests;
