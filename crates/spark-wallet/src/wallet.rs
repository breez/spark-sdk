use std::{collections::HashMap, str::FromStr, sync::Arc, time::Duration};

use bitcoin::{
    Address, Transaction,
    address::NetworkUnchecked,
    hashes::sha256::Hash,
    key::Secp256k1,
    secp256k1::{PublicKey, ecdsa::Signature},
};

use spark::{
    address::{
        SatsPayment, SparkAddress, SparkAddressPaymentType, SparkInvoiceFields, TokensPayment,
    },
    bitcoin::BitcoinService,
    events::{SparkEvent, subscribe_server_events},
    operator::{
        OperatorPool,
        rpc::{
            ConnectionManager, DefaultConnectionManager, OperatorRpcError,
            spark::{PreimageRequestRole, QuerySparkInvoicesRequest, UpdateWalletSettingRequest},
        },
    },
    services::{
        CoopExitFeeQuote, CoopExitParams, CoopExitService, CpfpUtxo, DepositService, ExitSpeed,
        Fee, FreezeIssuerTokenResponse, HtlcService, InvoiceDescription, LeafOptimizer,
        LeafTxCpfpPsbts, LightningReceivePayment, LightningSendPayment, LightningService,
        OptimizationEvent, OptimizationEventHandler, OptimizationProgress, Preimage,
        PreimageRequestStatus, PreimageRequestWithTransfer, QueryHtlcFilter,
        QueryTokenTransactionsFilter, ServiceError, StaticDepositQuote, Swap, TimelockManager,
        TokenService, TokenTransaction, Transfer, TransferId, TransferObserver, TransferService,
        TransferStatus, TransferTokenOutput, TransferType, UnilateralExitService, Utxo,
    },
    session_manager::{InMemorySessionManager, SessionManager},
    signer::Signer,
    ssp::{ServiceProvider, SspTransfer, SspUserRequest},
    token::{
        InMemoryTokenOutputStore, SelectionStrategy, SynchronousTokenOutputService, TokenMetadata,
        TokenOutputService, TokenOutputStore, TokenOutputWithPrevOut,
    },
    tree::{
        InMemoryTreeStore, SelectLeavesOptions, SynchronousTreeService, TargetAmounts, TreeNode,
        TreeNodeId, TreeService, TreeStore, select_leaves_by_target_amounts, with_reserved_leaves,
    },
    utils::paging::{PagingFilter, PagingResult},
};
use tokio::sync::{broadcast, watch};
use tokio_with_wasm::alias as tokio;
use tracing::{debug, error, info, trace};
use web_time::{SystemTime, UNIX_EPOCH};

use crate::{
    FulfillSparkInvoiceResult, ListTokenTransactionsRequest, ListTransfersRequest, PreimageRequest,
    QuerySparkInvoiceResult, TokenBalance, WalletEvent, WalletLeaves, WalletSettings,
    WithdrawInnerParams,
    event::EventManager,
    model::{PayLightningInvoiceResult, WalletInfo, WalletLeaf, WalletTransfer},
};

const SELECT_LEAVES_MAX_RETRIES: usize = 3;
const MAX_LEAF_SPENT_RETRIES: usize = 3;
const MAX_RATE_LIMIT_RETRIES: u32 = 5;
const RATE_LIMIT_BASE_DELAY_MS: u64 = 100;
const RATE_LIMIT_MAX_DELAY_MS: u64 = 3000;

use super::{SparkWalletConfig, SparkWalletError, TokenOutputsOptimizationOptions};

/// Checks if an error indicates a transfer lock conflict that should trigger a retry.
// TODO: We want to move to an error code but it isn't available yet.
fn is_transfer_locked_error<E: std::fmt::Display>(error: &E) -> bool {
    error.to_string().contains("TRANSFER_LOCKED")
}

/// Checks if an error is a gRPC PermissionDenied status, which can occur
/// when we are trying to spend leaves that have already been spent.
fn is_permission_denied_error(error: &OperatorRpcError) -> bool {
    matches!(error, OperatorRpcError::Connection(status) if status.code() == tonic::Code::PermissionDenied)
}

/// Checks if an error should trigger a retry of the operation.
fn is_leafs_spent_error(error: &OperatorRpcError) -> bool {
    is_transfer_locked_error(error) || is_permission_denied_error(error)
}

/// Checks if an error is a gRPC ResourceExhausted status, which occurs
/// when the operator rate limits or concurrency limits requests.
fn is_resource_exhausted_error(error: &OperatorRpcError) -> bool {
    matches!(error, OperatorRpcError::Connection(status) if status.code() == tonic::Code::ResourceExhausted)
}

/// Macro to handle retry logic for operations that may fail due to concurrent leaf spending.
/// This retries the operation up to MAX_LEAF_SPENT_RETRIES times.
macro_rules! with_leafs_spent_retry {
    (
        $self:expr,
        $target_amounts:expr,
        $operation_name:literal,
        |$leaves_reservation:ident| $operation:expr
    ) => {{
        let mut attempt = 0;
        let mut rate_limit_attempt: u32 = 0;
        loop {
            if attempt > 0 {
                if attempt >= MAX_LEAF_SPENT_RETRIES {
                    break Err(SparkWalletError::Generic(format!(
                        "{} failed after {} retries due to leaf spending errors",
                        $operation_name, MAX_LEAF_SPENT_RETRIES
                    )));
                }
                info!(
                    "{} failed with leaf spending error (attempt {}/{}), refreshing leaves and retrying",
                    $operation_name,
                    attempt,
                    MAX_LEAF_SPENT_RETRIES
                );
                $self.tree_service.refresh_leaves().await?;
            }
            let $leaves_reservation = $self.select_leaves_with_retry($target_amounts).await?;

            let result = with_reserved_leaves(
                $self.tree_service.as_ref(),
                $operation,
                &$leaves_reservation,
            )
            .await;

            match result {
                Ok(v) => break Ok(v),
                Err(ServiceError::ServiceConnectionError(e))
                    if is_resource_exhausted_error(&e) =>
                {
                    rate_limit_attempt += 1;
                    if rate_limit_attempt > MAX_RATE_LIMIT_RETRIES {
                        break Err(ServiceError::ServiceConnectionError(e).into());
                    }
                    let delay_ms = (RATE_LIMIT_BASE_DELAY_MS
                        * 2u64.pow(rate_limit_attempt - 1))
                    .min(RATE_LIMIT_MAX_DELAY_MS);
                    info!(
                        "{} rate limited (attempt {}/{}), backing off {}ms",
                        $operation_name, rate_limit_attempt,
                        MAX_RATE_LIMIT_RETRIES, delay_ms,
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    continue;
                }
                Err(ServiceError::ServiceConnectionError(e)) if is_leafs_spent_error(&e) => {
                    attempt += 1;
                    continue;
                }
                Err(e) => break Err(e.into()),
            }
        }
    }};
}

pub struct SparkWallet {
    /// Cancellation token sender for internal use. When dropped, background tasks will stop.
    /// This is `Some` only when no external cancellation token was provided.
    #[allow(dead_code)]
    cancel: Option<watch::Sender<()>>,
    config: SparkWalletConfig,
    deposit_service: Arc<DepositService>,
    event_manager: Arc<EventManager>,
    identity_public_key: PublicKey,
    signer: Arc<dyn Signer>,
    tree_service: Arc<dyn TreeService>,
    token_output_service: Arc<dyn TokenOutputService>,
    coop_exit_service: Arc<CoopExitService>,
    unilateral_exit_service: Arc<UnilateralExitService>,
    transfer_service: Arc<TransferService>,
    lightning_service: Arc<LightningService>,
    ssp_client: Arc<ServiceProvider>,
    token_service: Arc<TokenService>,
    operator_pool: Arc<OperatorPool>,
    htlc_service: Arc<HtlcService>,
    leaf_optimizer: Arc<LeafOptimizer>,
}

impl SparkWallet {
    pub async fn connect(
        config: SparkWalletConfig,
        signer: Arc<dyn Signer>,
    ) -> Result<Self, SparkWalletError> {
        Self::new(
            config,
            signer,
            Arc::new(InMemorySessionManager::default()),
            Arc::new(InMemoryTreeStore::default()),
            Arc::new(InMemoryTokenOutputStore::default()),
            Arc::new(DefaultConnectionManager::new()),
            None,
            true,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        config: SparkWalletConfig,
        signer: Arc<dyn Signer>,
        session_manager: Arc<dyn SessionManager>,
        tree_store: Arc<dyn TreeStore>,
        token_output_store: Arc<dyn TokenOutputStore>,
        connection_manager: Arc<dyn ConnectionManager>,
        transfer_observer: Option<Arc<dyn TransferObserver>>,
        with_background_processing: bool,
        cancellation_token: Option<watch::Receiver<()>>,
    ) -> Result<Self, SparkWalletError> {
        config.validate()?;
        let identity_public_key = signer.get_identity_public_key().await?;

        let bitcoin_service = BitcoinService::new(config.network);
        let service_provider = Arc::new(ServiceProvider::new(
            config.service_provider_config.clone(),
            signer.clone(),
            session_manager.clone(),
        ));

        let operator_pool = Arc::new(
            OperatorPool::connect(
                &config.operator_pool,
                connection_manager,
                Arc::clone(&session_manager),
                Arc::clone(&signer),
            )
            .await?,
        );

        let transfer_service = Arc::new(TransferService::new(
            signer.clone(),
            config.network,
            config.split_secret_threshold,
            operator_pool.clone(),
            transfer_observer.clone(),
        ));

        let lightning_service = Arc::new(LightningService::new(
            operator_pool.clone(),
            service_provider.clone(),
            config.network,
            Arc::clone(&signer),
            transfer_service.clone(),
            config.split_secret_threshold,
            transfer_observer.clone(),
        ));

        let timelock_manager = Arc::new(TimelockManager::new(
            signer.clone(),
            config.network,
            operator_pool.clone(),
        ));

        let deposit_service = Arc::new(DepositService::new(
            bitcoin_service,
            identity_public_key,
            config.network,
            operator_pool.clone(),
            service_provider.clone(),
            Arc::clone(&signer),
        ));

        let coop_exit_service = Arc::new(CoopExitService::new(
            operator_pool.clone(),
            service_provider.clone(),
            Arc::clone(&transfer_service),
            config.network,
            Arc::clone(&signer),
            transfer_observer.clone(),
        ));
        let unilateral_exit_service = Arc::new(UnilateralExitService::new(
            operator_pool.clone(),
            config.network,
        ));

        let swap_service = Arc::new(Swap::new(
            config.network,
            operator_pool.clone(),
            Arc::clone(&signer),
            Arc::clone(&service_provider),
            Arc::clone(&transfer_service),
        ));

        let tree_service: Arc<dyn TreeService> = Arc::new(SynchronousTreeService::new(
            identity_public_key,
            config.network,
            operator_pool.clone(),
            tree_store.clone(),
            Arc::clone(&timelock_manager),
            Arc::clone(&signer),
            Arc::clone(&swap_service),
        ));

        let token_output_service: Arc<dyn TokenOutputService> =
            Arc::new(SynchronousTokenOutputService::new(
                config.network,
                operator_pool.clone(),
                token_output_store,
                Arc::clone(&signer),
            ));

        let token_service = Arc::new(TokenService::new(
            token_output_service.clone(),
            Arc::clone(&signer),
            operator_pool.clone(),
            config.network,
            config.split_secret_threshold,
            config.tokens_config.clone(),
            transfer_observer.clone(),
        ));

        let htlc_service = Arc::new(HtlcService::new(
            operator_pool.clone(),
            config.network,
            Arc::clone(&signer),
            Arc::clone(&transfer_service),
            transfer_observer,
        ));

        let event_manager = Arc::new(EventManager::new());

        // Create optimization event handler that bridges to WalletEvent
        let optimization_event_handler = Arc::new(WalletOptimizationEventHandler {
            event_manager: Arc::clone(&event_manager),
        });

        let leaf_optimizer = Arc::new(LeafOptimizer::new(
            config.leaf_optimization_options.clone(),
            Arc::clone(&swap_service),
            Arc::clone(&tree_service),
            Some(optimization_event_handler),
        ));

        // Use external cancellation token if provided, otherwise create an internal one
        let (cancel, cancellation_token) = match cancellation_token {
            Some(token) => (None, token),
            None => {
                let (sender, receiver) = watch::channel(());
                (Some(sender), receiver)
            }
        };

        if with_background_processing {
            let reconnect_interval = Duration::from_secs(config.reconnect_interval_seconds);
            let background_processor = Arc::new(BackgroundProcessor::new(
                Arc::clone(&operator_pool),
                Arc::clone(&event_manager),
                identity_public_key,
                reconnect_interval,
                Arc::clone(&tree_service),
                Arc::clone(&service_provider),
                Arc::clone(&transfer_service),
                Arc::clone(&htlc_service),
                Arc::clone(&leaf_optimizer),
                config.leaf_auto_optimize_enabled,
                Arc::clone(&token_service),
                config.token_outputs_optimization_options.clone(),
            ));
            background_processor
                .run_background_tasks(cancellation_token.clone())
                .await;
        }
        Ok(Self {
            cancel,
            config,
            deposit_service,
            event_manager,
            identity_public_key,
            signer,
            tree_service,
            token_output_service,
            coop_exit_service,
            unilateral_exit_service,
            transfer_service,
            lightning_service,
            ssp_client: service_provider.clone(),
            token_service,
            operator_pool,
            htlc_service,
            leaf_optimizer,
        })
    }
}

impl SparkWallet {
    pub fn get_identity_public_key(&self) -> PublicKey {
        self.identity_public_key
    }

    pub async fn list_leaves(&self) -> Result<WalletLeaves, SparkWalletError> {
        let leaves = self.tree_service.list_leaves().await?;
        Ok(leaves.into())
    }

    /// Starts leaf optimization if auto-optimization is enabled.
    fn maybe_start_optimization(&self) {
        if self.config.leaf_auto_optimize_enabled {
            self.leaf_optimizer.start();
        }
    }

    pub async fn pay_lightning_invoice(
        &self,
        invoice: &str,
        amount_to_send: Option<u64>,
        max_fee_sat: Option<u64>,
        prefer_spark: bool,
        transfer_id: Option<TransferId>,
    ) -> Result<PayLightningInvoiceResult, SparkWalletError> {
        let (total_amount_sat, receiver_spark_address) = self
            .lightning_service
            .validate_payment(invoice, max_fee_sat, amount_to_send, prefer_spark)
            .await?;

        // In case the invoice is for a spark address, we can just transfer the amount to the receiver.
        if let Some(receiver_spark_address) = receiver_spark_address {
            if !self.config.self_payment_allowed
                && receiver_spark_address.identity_public_key == self.identity_public_key
            {
                return Err(SparkWalletError::SelfPaymentNotAllowed);
            }

            return Ok(PayLightningInvoiceResult {
                transfer: self
                    .transfer(total_amount_sat, &receiver_spark_address, transfer_id)
                    .await?,
                lightning_payment: None,
            });
        }

        let target_amounts = TargetAmounts::new_amount_and_fee(total_amount_sat, None);

        // Start the lightning swap with the operator, with retry logic for concurrent leaf spending
        let lightning_payment = with_leafs_spent_retry!(
            self,
            Some(&target_amounts),
            "Lightning payment",
            |leaves_reservation| self.lightning_service.pay_lightning_invoice(
                invoice,
                amount_to_send,
                &leaves_reservation.leaves,
                transfer_id.clone(),
            )
        )?;

        // Collect the wallet transfer information from the lightning send payment result. If
        // not present, we need to query for the SSP user request to get the transfer details.
        let payment_hash = lightning_payment.payment_hash;
        let lightning_send_payment = lightning_payment.lightning_send_payment;
        let wallet_transfer = match &lightning_send_payment {
            Some(lsp) => {
                let preimage = lsp
                    .payment_preimage
                    .as_deref()
                    .map(Preimage::from_hex)
                    .transpose()
                    .map_err(SparkWalletError::ServiceError)?;
                let preimage_request = PreimageRequest {
                    payment_hash,
                    status: if preimage.is_some() {
                        PreimageRequestStatus::PreimageShared
                    } else {
                        PreimageRequestStatus::WaitingForPreimage
                    },
                    created_time: UNIX_EPOCH
                        + Duration::from_secs(
                            lightning_payment.transfer.created_time.unwrap_or_default(),
                        ),
                    expiry_time: UNIX_EPOCH
                        + Duration::from_secs(
                            lightning_payment.transfer.expiry_time.unwrap_or_default(),
                        ),
                    preimage,
                };
                WalletTransfer::from_transfer(
                    lightning_payment.transfer,
                    None,
                    Some(preimage_request),
                    self.identity_public_key,
                    self.config.service_provider_config.identity_public_key,
                )
            }
            None => {
                create_transfer(
                    lightning_payment.transfer,
                    &self.ssp_client,
                    &self.htlc_service,
                    self.identity_public_key,
                    self.config.service_provider_config.identity_public_key,
                )
                .await?
            }
        };

        self.maybe_start_optimization();

        Ok(PayLightningInvoiceResult {
            transfer: wallet_transfer,
            lightning_payment: lightning_send_payment,
        })
    }

    /// Creates a Lightning invoice for the specified amount and description.
    /// If a public key is provided, the invoice will be associated with that key.
    /// Otherwise, the wallet's identity public key will be used.
    pub async fn create_lightning_invoice(
        &self,
        amount_sat: u64,
        description: Option<InvoiceDescription>,
        public_key: Option<PublicKey>,
        expiry_secs: Option<u32>,
        include_spark_address: bool,
    ) -> Result<LightningReceivePayment, SparkWalletError> {
        Ok(self
            .lightning_service
            .create_lightning_invoice(
                amount_sat,
                description,
                None,
                expiry_secs,
                include_spark_address,
                public_key,
            )
            .await?)
    }

    /// Creates a HODL Lightning invoice. The SSP will hold the HTLC until
    /// `claim_htlc` is called with the preimage matching the payment_hash.
    pub async fn create_hodl_lightning_invoice(
        &self,
        amount_sat: u64,
        description: Option<InvoiceDescription>,
        payment_hash: Hash,
        public_key: Option<PublicKey>,
        expiry_secs: Option<u32>,
    ) -> Result<LightningReceivePayment, SparkWalletError> {
        Ok(self
            .lightning_service
            .create_hodl_lightning_invoice(
                amount_sat,
                description,
                payment_hash,
                expiry_secs,
                public_key,
            )
            .await?)
    }

    pub async fn fetch_coop_exit_fee_quote(
        &self,
        withdrawal_address: &str,
        amount_sats: Option<u64>,
    ) -> Result<CoopExitFeeQuote, SparkWalletError> {
        // Validate withdrawal address
        let withdrawal_address = withdrawal_address
            .parse::<Address<NetworkUnchecked>>()
            .map_err(|_| {
                SparkWalletError::InvalidAddress(format!(
                    "Invalid withdrawal address: {withdrawal_address}"
                ))
            })?
            .require_network(self.config.network.into())
            .map_err(|_| SparkWalletError::InvalidNetwork)?;

        // Selects leaves totaling `amount_sat` if provided, otherwise retrieves all available leaves.
        let target_amounts =
            amount_sats.map(|amount| TargetAmounts::new_amount_and_fee(amount, None));
        let reservation = self
            .select_leaves_with_retry(target_amounts.as_ref())
            .await?;

        // Fetches fee quote for the coop exit then cancels the reservation.
        let fee_quote_res = self
            .coop_exit_service
            .fetch_coop_exit_fee_quote(reservation.leaves, withdrawal_address)
            .await;
        self.tree_service.cancel_reservation(reservation.id).await?;

        Ok(fee_quote_res?)
    }

    pub async fn fetch_lightning_send_fee_estimate(
        &self,
        invoice: &str,
        amount_to_send: Option<u64>,
    ) -> Result<u64, SparkWalletError> {
        Ok(self
            .lightning_service
            .fetch_lightning_send_fee_estimate(invoice, amount_to_send)
            .await?)
    }

    pub async fn fetch_lightning_send_payment(
        &self,
        id: &str,
    ) -> Result<Option<LightningSendPayment>, SparkWalletError> {
        Ok(self
            .lightning_service
            .get_lightning_send_payment(id)
            .await?)
    }

    pub fn extract_spark_address(
        &self,
        invoice: &str,
    ) -> Result<Option<SparkAddress>, SparkWalletError> {
        Ok(self
            .lightning_service
            .extract_spark_address_from_invoice(invoice)?)
    }

    pub async fn fetch_lightning_receive_payment(
        &self,
        id: &str,
    ) -> Result<Option<LightningReceivePayment>, SparkWalletError> {
        Ok(self
            .lightning_service
            .get_lightning_receive_payment(id)
            .await?)
    }

    pub async fn get_utxos_for_address(
        &self,
        address: &str,
    ) -> Result<Vec<Utxo>, SparkWalletError> {
        Ok(self.deposit_service.get_utxos_for_address(address).await?)
    }

    // TODO: In the js sdk this function calls an electrum server to fetch the transaction hex based on a txid.
    // Intuitively this function is being called when you've already learned about a transaction, so it could be passed in directly.
    /// Claims a deposit by finding the first unused deposit address in the transaction outputs.
    pub async fn claim_deposit(
        &self,
        tx: Transaction,
        vout: u32,
    ) -> Result<Vec<WalletLeaf>, SparkWalletError> {
        let deposit_nodes = self.deposit_service.claim_deposit(tx, vout).await?;
        self.tree_service
            .insert_leaves(deposit_nodes.clone())
            .await?;
        info!("Claimed deposit root node: {:?}", deposit_nodes);

        self.maybe_start_optimization();

        Ok(deposit_nodes.into_iter().map(WalletLeaf::from).collect())
    }

    pub async fn claim_static_deposit(
        &self,
        quote: StaticDepositQuote,
    ) -> Result<WalletTransfer, SparkWalletError> {
        let transfer = self.deposit_service.claim_static_deposit(quote).await?;

        self.maybe_start_optimization();

        Ok(WalletTransfer::from_transfer(
            transfer,
            None,
            None,
            self.identity_public_key,
            self.config.service_provider_config.identity_public_key,
        ))
    }

    pub async fn refund_static_deposit(
        &self,
        tx: Transaction,
        output_index: Option<u32>,
        refund_address: &str,
        fee: Fee,
    ) -> Result<Transaction, SparkWalletError> {
        let refund_address = refund_address
            .parse::<Address<NetworkUnchecked>>()
            .map_err(|_| {
                SparkWalletError::InvalidAddress(format!(
                    "Invalid refund address: {refund_address}"
                ))
            })?
            .require_network(self.config.network.into())
            .map_err(|_| SparkWalletError::InvalidNetwork)?;

        let refund_tx = self
            .deposit_service
            .refund_static_deposit(tx, output_index, refund_address, fee)
            .await?;

        Ok(refund_tx)
    }

    pub async fn generate_deposit_address(
        &self,
        is_static: bool,
    ) -> Result<Address, SparkWalletError> {
        let leaf_id = TreeNodeId::generate();
        let signing_public_key = if is_static {
            self.signer.static_deposit_signing_key(0).await?
        } else {
            self.signer.get_public_key_for_node(&leaf_id).await?
        };
        let address = self
            .deposit_service
            .generate_deposit_address(signing_public_key, &leaf_id, is_static)
            .await?;

        // TODO: Watch this address for deposits.

        Ok(address.address)
    }

    pub async fn list_static_deposit_addresses(
        &self,
        paging: Option<PagingFilter>,
    ) -> Result<PagingResult<Address>, SparkWalletError> {
        let static_addresses = self
            .deposit_service
            .query_static_deposit_addresses(paging)
            .await?;
        Ok(static_addresses.map(|addr| addr.address))
    }

    pub async fn list_unused_deposit_addresses(
        &self,
        paging: Option<PagingFilter>,
    ) -> Result<PagingResult<Address>, SparkWalletError> {
        let deposit_addresses = self
            .deposit_service
            .query_unused_deposit_addresses(paging)
            .await?;
        Ok(deposit_addresses.map(|addr| addr.address))
    }

    /// Fetches a quote for the creditable amount when claiming a static deposit.
    pub async fn fetch_static_deposit_claim_quote(
        &self,
        tx: Transaction,
        output_index: Option<u32>,
    ) -> Result<StaticDepositQuote, SparkWalletError> {
        Ok(self
            .deposit_service
            .fetch_static_deposit_claim_quote(tx, output_index)
            .await?)
    }

    /// Sends a transfer to another Spark user.
    pub async fn transfer(
        &self,
        amount_sat: u64,
        receiver_address: &SparkAddress,
        transfer_id: Option<TransferId>,
    ) -> Result<WalletTransfer, SparkWalletError> {
        if receiver_address.is_invoice() {
            return Err(SparkWalletError::Generic(
                "Receiver address is a Spark invoice. Use `fulfill_spark_invoice` instead."
                    .to_string(),
            ));
        }

        self.transfer_with_invoice(amount_sat, receiver_address, transfer_id, None)
            .await
    }

    async fn transfer_with_invoice(
        &self,
        amount_sat: u64,
        receiver_address: &SparkAddress,
        transfer_id: Option<TransferId>,
        spark_invoice: Option<String>,
    ) -> Result<WalletTransfer, SparkWalletError> {
        // validate receiver address and get its pubkey
        if self.config.network != receiver_address.network {
            return Err(SparkWalletError::InvalidNetwork);
        }

        if !self.config.self_payment_allowed
            && receiver_address.identity_public_key == self.identity_public_key
        {
            return Err(SparkWalletError::SelfPaymentNotAllowed);
        }

        // Transfer leaves with retry logic for concurrent leaf spending
        let target_amounts = TargetAmounts::new_amount_and_fee(amount_sat, None);
        let transfer = with_leafs_spent_retry!(
            self,
            Some(&target_amounts),
            "Transfer",
            |leaves_reservation| self.transfer_service.transfer_leaves_to(
                leaves_reservation.leaves.clone(),
                &receiver_address.identity_public_key,
                transfer_id.clone(),
                None,
                spark_invoice.clone(),
            )
        )?;

        self.maybe_start_optimization();

        Ok(WalletTransfer::from_transfer(
            transfer,
            None,
            None,
            self.identity_public_key,
            self.config.service_provider_config.identity_public_key,
        ))
    }

    /// Claims all pending transfers.
    pub async fn claim_pending_transfers(&self) -> Result<Vec<WalletTransfer>, SparkWalletError> {
        let transfers = claim_pending_transfers(
            self.identity_public_key,
            &self.transfer_service,
            &self.tree_service,
            &self.htlc_service,
            &self.ssp_client,
        )
        .await?;

        if !transfers.is_empty() {
            self.maybe_start_optimization();
        }

        for transfer in &transfers {
            self.event_manager
                .notify_listeners(WalletEvent::TransferClaimed(transfer.clone()));
        }

        Ok(transfers)
    }

    pub async fn create_htlc(
        &self,
        amount_sat: u64,
        receiver_address: &SparkAddress,
        payment_hash: &Hash,
        expiry_duration: Duration,
        transfer_id: Option<TransferId>,
    ) -> Result<WalletTransfer, SparkWalletError> {
        // validate receiver address and get its pubkey
        if self.config.network != receiver_address.network {
            return Err(SparkWalletError::InvalidNetwork);
        }

        if !self.config.self_payment_allowed
            && receiver_address.identity_public_key == self.identity_public_key
        {
            return Err(SparkWalletError::SelfPaymentNotAllowed);
        }

        // Create HTLC with retry logic for concurrent leaf spending
        let target_amounts = TargetAmounts::new_amount_and_fee(amount_sat, None);
        let expiry_time = SystemTime::now() + expiry_duration;
        let transfer = with_leafs_spent_retry!(
            self,
            Some(&target_amounts),
            "HTLC creation",
            |leaves_reservation| self.htlc_service.create_htlc(
                leaves_reservation.leaves.clone(),
                &receiver_address.identity_public_key,
                payment_hash,
                expiry_time,
                transfer_id.clone(),
            )
        )?;

        let htlc_preimage_request = PreimageRequest {
            payment_hash: *payment_hash,
            status: PreimageRequestStatus::WaitingForPreimage,
            created_time: transfer
                .created_time
                .map(|t| UNIX_EPOCH + Duration::from_secs(t))
                .unwrap_or(SystemTime::now()),
            expiry_time,
            preimage: None,
        };

        Ok(WalletTransfer::from_transfer(
            transfer,
            None,
            Some(htlc_preimage_request),
            self.identity_public_key,
            self.config.service_provider_config.identity_public_key,
        ))
    }

    pub async fn claim_htlc(
        &self,
        preimage: &Preimage,
    ) -> Result<WalletTransfer, SparkWalletError> {
        let transfer = self.htlc_service.provide_preimage(preimage).await?;

        // Fetch HTLC preimage request data
        let preimage_request = self
            .htlc_service
            .query_htlc(
                QueryHtlcFilter {
                    payment_hashes: vec![preimage.compute_hash().to_string()],
                    identity_public_key: self.identity_public_key,
                    status: None,
                    transfer_ids: Vec::new(),
                    match_role: PreimageRequestRole::Receiver,
                },
                None,
            )
            .await?
            .items
            .first()
            .cloned()
            .ok_or(SparkWalletError::Generic("HTLC not found".to_string()))?;

        // Also fetch SSP transfer data so Lightning payments get user_request
        let ssp_transfer = self
            .ssp_client
            .get_transfers(vec![transfer.id.to_string()])
            .await?
            .into_iter()
            .next();

        Ok(WalletTransfer::from_transfer(
            transfer,
            ssp_transfer,
            Some(preimage_request.into()),
            self.identity_public_key,
            self.config.service_provider_config.identity_public_key,
        ))
    }

    pub async fn list_claimable_htlc_transfers(
        &self,
        paging: Option<PagingFilter>,
    ) -> Result<Vec<WalletTransfer>, SparkWalletError> {
        let htlcs = self
            .htlc_service
            .query_htlc(
                QueryHtlcFilter {
                    identity_public_key: self.identity_public_key,
                    status: Some(PreimageRequestStatus::WaitingForPreimage),
                    transfer_ids: Vec::new(),
                    payment_hashes: Vec::new(),
                    match_role: PreimageRequestRole::Receiver,
                },
                paging,
            )
            .await?
            .items;
        htlcs
            .into_iter()
            .map(|h| {
                WalletTransfer::from_preimage_request_with_transfer(
                    h,
                    self.identity_public_key,
                    self.config.service_provider_config.identity_public_key,
                )
            })
            .collect()
    }

    pub fn get_info(&self) -> WalletInfo {
        WalletInfo {
            identity_public_key: self.identity_public_key,
            network: self.config.network,
        }
    }

    pub fn get_spark_address(&self) -> Result<SparkAddress, SparkWalletError> {
        Ok(SparkAddress::new(
            self.identity_public_key,
            self.config.network,
            None,
        ))
    }

    pub async fn create_spark_invoice(
        &self,
        amount: Option<u128>,
        token_identifier: Option<String>,
        expiry_time: Option<SystemTime>,
        description: Option<String>,
        sender_public_key: Option<PublicKey>,
    ) -> Result<String, SparkWalletError> {
        let payment_type = if let Some(token_identifier) = token_identifier {
            SparkAddressPaymentType::TokensPayment(TokensPayment {
                token_identifier: Some(token_identifier),
                amount,
            })
        } else {
            SparkAddressPaymentType::SatsPayment(SatsPayment {
                amount: amount
                    .map(|amount| amount.try_into())
                    .transpose()
                    .map_err(|_| SparkWalletError::Generic("Invalid sats amount".to_string()))?,
            })
        };

        let invoice_fields = SparkInvoiceFields {
            id: uuid::Uuid::now_v7(),
            version: 1,
            memo: description,
            sender_public_key,
            expiry_time,
            payment_type: Some(payment_type),
        };

        let invoice = SparkAddress::new(
            self.identity_public_key,
            self.config.network,
            Some(invoice_fields),
        );

        Ok(invoice.to_invoice_string(&*self.signer).await?)
    }

    pub async fn get_balance(&self) -> Result<u64, SparkWalletError> {
        Ok(self.tree_service.get_available_balance().await?)
    }

    pub async fn list_transfers(
        &self,
        request: ListTransfersRequest,
    ) -> Result<PagingResult<WalletTransfer>, SparkWalletError> {
        let our_pubkey = self.identity_public_key;
        let transfers = self
            .transfer_service
            .query_transfers(&request.transfer_ids, request.paging)
            .await?;
        create_transfers(
            transfers,
            &self.ssp_client,
            &self.htlc_service,
            our_pubkey,
            self.ssp_client.identity_public_key(),
        )
        .await
    }

    pub async fn list_pending_transfers(
        &self,
        paging: Option<PagingFilter>,
    ) -> Result<PagingResult<WalletTransfer>, SparkWalletError> {
        let our_pubkey = self.identity_public_key;
        let transfers = self
            .transfer_service
            .query_pending_transfers(paging)
            .await?;
        create_transfers(
            transfers,
            &self.ssp_client,
            &self.htlc_service,
            our_pubkey,
            self.ssp_client.identity_public_key(),
        )
        .await
    }

    /// Queries the SSP for user requests by their associated transfer IDs
    /// and returns a map of transfer IDs to user requests
    pub async fn query_ssp_user_requests(
        &self,
        transfer_ids: Vec<String>,
    ) -> Result<HashMap<String, SspUserRequest>, SparkWalletError> {
        let transfers = self.ssp_client.get_transfers(transfer_ids).await?;
        Ok(transfers
            .into_iter()
            .filter_map(
                |transfer| match (transfer.spark_id, transfer.user_request) {
                    (Some(spark_id), Some(user_request)) => Some((spark_id, user_request)),
                    _ => None,
                },
            )
            .collect())
    }

    /// Signs a message with the identity key using ECDSA and returns the signature.
    ///
    /// If exposing this, consider adding a prefix to prevent mistakenly signing messages.
    pub async fn sign_message(&self, message: &str) -> Result<Signature, SparkWalletError> {
        Ok(self
            .signer
            .sign_message_ecdsa_with_identity_key(message.as_bytes())
            .await?)
    }

    /// Verifies a message was signed by the given public key and the signature is valid.
    pub async fn verify_message(
        &self,
        message: &str,
        signature: &Signature,
        public_key: &PublicKey,
    ) -> Result<(), SparkWalletError> {
        spark::utils::verify_signature::verify_signature_ecdsa(
            &Secp256k1::new(),
            message,
            signature,
            public_key,
        )
        .map_err(|e| SparkWalletError::ValidationError(e.to_string()))
    }

    pub async fn sync(&self) -> Result<(), SparkWalletError> {
        self.tree_service.refresh_leaves().await?;
        self.token_output_service.refresh_tokens_outputs().await?;
        // Claiming here any transfers that may have been missed in the event stream handling (e.g. counter swap transfers are intentionally not claimed there)
        self.claim_pending_transfers().await?;
        Ok(())
    }

    pub async fn withdraw(
        &self,
        withdrawal_address: &str,
        amount_sats: Option<u64>,
        exit_speed: ExitSpeed,
        fee_quote: CoopExitFeeQuote,
        transfer_id: Option<TransferId>,
    ) -> Result<WalletTransfer, SparkWalletError> {
        // Validate withdrawal address
        let withdrawal_address = withdrawal_address
            .parse::<Address<NetworkUnchecked>>()
            .map_err(|_| {
                SparkWalletError::InvalidAddress(format!(
                    "Invalid withdrawal address: {withdrawal_address}"
                ))
            })?
            .require_network(self.config.network.into())
            .map_err(|_| SparkWalletError::InvalidNetwork)?;

        // Calculate the fee based on the exit speed
        let fee_sats = fee_quote.fee_sats(&exit_speed);
        trace!("Calculated fee for exit speed {exit_speed:?}: {fee_sats} sats",);

        // Withdraw leaves with retry logic for concurrent leaf spending
        let target_amounts = amount_sats
            .map(|amount_sats| TargetAmounts::new_amount_and_fee(amount_sats, Some(fee_sats)));
        let transfer = with_leafs_spent_retry!(
            self,
            target_amounts.as_ref(),
            "Withdrawal",
            |leaves_reservation| self.withdraw_inner(WithdrawInnerParams {
                address: withdrawal_address.clone(),
                exit_speed,
                leaves_reservation: &leaves_reservation,
                target_amounts: target_amounts.as_ref(),
                fee_sats,
                fee_quote_id: fee_quote.id.clone(),
                transfer_id: transfer_id.clone(),
            })
        )?;

        self.maybe_start_optimization();

        create_transfer(
            transfer,
            &self.ssp_client,
            &self.htlc_service,
            self.identity_public_key,
            self.config.service_provider_config.identity_public_key,
        )
        .await
    }

    async fn withdraw_inner(
        &self,
        params: WithdrawInnerParams<'_>,
    ) -> Result<Transfer, ServiceError> {
        let WithdrawInnerParams {
            address,
            exit_speed,
            leaves_reservation,
            target_amounts,
            fee_sats,
            fee_quote_id,
            transfer_id,
        } = params;
        let withdraw_all = target_amounts.is_none();
        let (withdraw_leaves, fee_leaves, fee_quote_id) = if withdraw_all {
            (leaves_reservation.leaves.clone(), None, None)
        } else {
            let target_leaves =
                select_leaves_by_target_amounts(&leaves_reservation.leaves, target_amounts)?;
            (
                target_leaves.amount_leaves,
                target_leaves.fee_leaves,
                Some(fee_quote_id),
            )
        };

        // Check if the fee is greater than the amount when deducting the fee from it
        let withdraw_leaves_sum: u64 = withdraw_leaves.iter().map(|leaf| leaf.value).sum();
        if withdraw_all && fee_sats > withdraw_leaves_sum {
            trace!(
                "Insufficient funds for withdrawal: amount {} sats, fee {} sats",
                withdraw_leaves_sum, fee_sats
            );
            return Err(ServiceError::InsufficientFunds);
        }

        // Perform the cooperative exit with the SSP
        let transfer = self
            .coop_exit_service
            .coop_exit(CoopExitParams {
                leaves: withdraw_leaves,
                withdrawal_address: &address,
                withdraw_all,
                exit_speed,
                fee_quote_id,
                fee_leaves,
                transfer_id,
            })
            .await?;

        Ok(transfer)
    }

    /// Prepares a package of unilaterial exit PSBTs for each leaf
    ///
    /// # Arguments
    /// * `fee_rate` - The fee rate used to calculate the PSBT fee, in satoshis per vbyte
    /// * `leaf_ids` - The IDs of the leaves to unilaterally exit
    /// * `utxos` - The UTXOs to use as inputs for the PSBTs. Currently only supports p2wpkh addresses
    pub async fn unilateral_exit(
        &self,
        fee_rate: u64,
        leaf_ids: Vec<TreeNodeId>,
        utxos: Vec<CpfpUtxo>,
    ) -> Result<Vec<LeafTxCpfpPsbts>, SparkWalletError> {
        Ok(self
            .unilateral_exit_service
            .unilateral_exit(fee_rate, leaf_ids, utxos)
            .await?)
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<WalletEvent> {
        self.event_manager.listen()
    }

    /// Returns the balances of all tokens in the wallet.
    ///
    /// Balances are returned in a map keyed by the token identifier.
    pub async fn get_token_balances(
        &self,
    ) -> Result<HashMap<String, TokenBalance>, SparkWalletError> {
        let token_outputs = self.token_output_service.list_tokens_outputs().await?;

        let balances = token_outputs
            .into_iter()
            .map(|token_outputs| {
                let balance = token_outputs.balance();
                (
                    token_outputs.metadata.identifier.clone(),
                    TokenBalance {
                        balance,
                        token_metadata: token_outputs.metadata,
                    },
                )
            })
            .collect();

        Ok(balances)
    }

    /// Transfers tokens to another Spark user.
    ///
    /// Multiple outputs may be provided but they must share the same token id.
    pub async fn transfer_tokens(
        &self,
        outputs: Vec<TransferTokenOutput>,
        selected_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
    ) -> Result<TokenTransaction, SparkWalletError> {
        if outputs.iter().any(|o| o.spark_invoice.is_some()) {
            return Err(SparkWalletError::Generic(
                "Spark invoices are not supported for token transfers. Use the `fulfill_spark_invoice` method instead.".to_string(),
            ));
        }

        if !self.config.self_payment_allowed
            && outputs
                .iter()
                .any(|o| o.receiver_address.identity_public_key == self.identity_public_key)
        {
            return Err(SparkWalletError::SelfPaymentNotAllowed);
        }

        let tx = self
            .token_service
            .transfer_tokens(outputs, selected_outputs, selection_strategy)
            .await?;
        Ok(tx)
    }

    pub async fn list_token_transactions(
        &self,
        request: ListTokenTransactionsRequest,
    ) -> Result<PagingResult<TokenTransaction>, SparkWalletError> {
        self.token_service
            .query_token_transactions(
                QueryTokenTransactionsFilter {
                    owner_public_keys: request.owner_public_keys,
                    issuer_public_keys: request.issuer_public_keys,
                    token_ids: request.token_ids,
                    output_ids: request.output_ids,
                },
                request.paging,
            )
            .await
            .map_err(Into::into)
    }

    /// Queries token transactions by their hashes.
    ///
    /// Limited to 100 hashes per request.
    pub async fn get_token_transactions_by_hashes(
        &self,
        hashes: Vec<String>,
    ) -> Result<Vec<TokenTransaction>, SparkWalletError> {
        self.token_service
            .query_token_transactions_by_hashes(hashes)
            .await
            .map_err(Into::into)
    }

    pub fn get_token_l1_address(&self) -> Result<String, SparkWalletError> {
        let compressed_pubkey =
            bitcoin::key::CompressedPublicKey::from_slice(&self.identity_public_key.serialize())
                .map_err(|e| SparkWalletError::ValidationError(e.to_string()))?;
        Ok(Address::p2wpkh(
            &compressed_pubkey,
            bitcoin::Network::from(self.config.network),
        )
        .to_string())
    }

    pub async fn get_tokens_metadata(
        &self,
        token_identifiers: &[&str],
        issuer_public_keys: &[PublicKey],
    ) -> Result<Vec<TokenMetadata>, SparkWalletError> {
        self.token_service
            .get_tokens_metadata(token_identifiers, issuer_public_keys)
            .await
            .map_err(Into::into)
    }

    pub async fn get_issuer_token_balance(&self) -> Result<TokenBalance, SparkWalletError> {
        let token_identifier = self.get_issuer_token_metadata().await?.identifier;
        let token_balances = self.get_token_balances().await?;

        Ok(token_balances
            .get(&token_identifier)
            .ok_or(SparkWalletError::Generic(
                "No issuer token found".to_string(),
            ))?
            .clone())
    }

    pub async fn get_issuer_token_metadata(&self) -> Result<TokenMetadata, SparkWalletError> {
        Ok(self.token_service.get_issuer_token_metadata().await?)
    }

    pub async fn create_issuer_token(
        &self,
        name: &str,
        ticker: &str,
        decimals: u32,
        is_freezable: bool,
        max_supply: u128,
    ) -> Result<TokenTransaction, SparkWalletError> {
        let token_transaction = self
            .token_service
            .create_issuer_token(name, ticker, decimals, is_freezable, max_supply)
            .await?;
        Ok(token_transaction)
    }

    pub async fn mint_issuer_token(
        &self,
        amount: u128,
    ) -> Result<TokenTransaction, SparkWalletError> {
        let token_transaction = self.token_service.mint_issuer_token(amount).await?;
        Ok(token_transaction)
    }

    pub async fn burn_issuer_token(
        &self,
        amount: u128,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
    ) -> Result<TokenTransaction, SparkWalletError> {
        let token_transaction = self
            .token_service
            .burn_issuer_token(amount, preferred_outputs, None)
            .await?;
        Ok(token_transaction)
    }

    pub async fn freeze_issuer_token(
        &self,
        spark_address: &SparkAddress,
    ) -> Result<FreezeIssuerTokenResponse, SparkWalletError> {
        Ok(self
            .token_service
            .freeze_issuer_token(spark_address, false)
            .await?)
    }

    pub async fn unfreeze_issuer_token(
        &self,
        spark_address: &SparkAddress,
    ) -> Result<FreezeIssuerTokenResponse, SparkWalletError> {
        Ok(self
            .token_service
            .freeze_issuer_token(spark_address, true)
            .await?)
    }

    /// Fulfills a Spark invoice by paying the requested asset (Bitcoin or token) and amount (optional).
    ///
    /// # Arguments
    /// * `invoice` - The Spark invoice to fulfill
    /// * `amount` - The amount to pay in base units. Must be provided if the invoice doesn't include an amount. If it does, amount is ignored.
    pub async fn fulfill_spark_invoice(
        &self,
        invoice_str: &str,
        amount: Option<u128>,
        transfer_id: Option<TransferId>,
    ) -> Result<FulfillSparkInvoiceResult, SparkWalletError> {
        let invoice = SparkAddress::from_str(invoice_str)?;

        let Some(invoice_fields) = &invoice.spark_invoice_fields else {
            return Err(SparkWalletError::InvalidAddress(format!(
                "Invoice does not include Spark invoice fields: {invoice:?}"
            )));
        };

        if let Some(expiry_time) = invoice_fields.expiry_time
            && expiry_time < SystemTime::now()
        {
            return Err(SparkWalletError::InvalidAddress(format!(
                "Invoice has expired at {}",
                expiry_time.duration_since(UNIX_EPOCH).unwrap().as_secs()
            )));
        }

        if let Some(sender_public_key) = invoice_fields.sender_public_key
            && sender_public_key != self.identity_public_key
        {
            return Err(SparkWalletError::InvalidAddress(format!(
                "Invoice sender public key does not match identity public key: {sender_public_key}"
            )));
        }

        match &invoice_fields.payment_type {
            Some(SparkAddressPaymentType::SatsPayment(payment)) => {
                let amount = payment.amount.or(amount.map(|a| a as u64)).ok_or(
                    SparkWalletError::Generic(
                        "Amount is required when invoice does not include an amount".to_string(),
                    ),
                )?;

                let transfer = self
                    .transfer_with_invoice(
                        amount,
                        &invoice,
                        transfer_id,
                        Some(invoice_str.to_string()),
                    )
                    .await?;

                Ok(FulfillSparkInvoiceResult::Transfer(Box::new(transfer)))
            }
            Some(SparkAddressPaymentType::TokensPayment(payment)) => {
                let Some(token_identifier) = &payment.token_identifier else {
                    return Err(SparkWalletError::InvalidAddress(
                        "Token invoice does not include token identifier".to_string(),
                    ));
                };
                let amount = payment.amount.or(amount).ok_or(SparkWalletError::Generic(
                    "Amount is required when invoice does not include an amount".to_string(),
                ))?;

                let tx = self
                    .token_service
                    .transfer_tokens(
                        vec![TransferTokenOutput {
                            token_id: token_identifier.clone(),
                            amount,
                            receiver_address: invoice,
                            spark_invoice: Some(invoice_str.to_string()),
                        }],
                        None,
                        None,
                    )
                    .await?;

                Ok(FulfillSparkInvoiceResult::TokenTransaction(Box::new(tx)))
            }
            None => Err(SparkWalletError::InvalidAddress(
                "Invoice does not include payment type".to_string(),
            )),
        }
    }

    pub async fn query_spark_invoices(
        &self,
        invoices: Vec<String>,
    ) -> Result<Vec<QuerySparkInvoiceResult>, SparkWalletError> {
        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .query_spark_invoices(QuerySparkInvoicesRequest {
                invoice: invoices,
                limit: 0,
                offset: 0,
            })
            .await?;

        response
            .invoice_statuses
            .into_iter()
            .map(TryInto::try_into)
            .collect()
    }

    pub async fn query_wallet_settings(&self) -> Result<WalletSettings, SparkWalletError> {
        Ok(self
            .operator_pool
            .get_coordinator()
            .client
            .query_wallet_setting()
            .await?
            .wallet_setting
            .ok_or(SparkWalletError::Generic(
                "Response doesn't include wallet settings".to_string(),
            ))?
            .into())
    }

    pub async fn update_wallet_settings(
        &self,
        private_enabled: bool,
    ) -> Result<(), SparkWalletError> {
        self.operator_pool
            .get_coordinator()
            .client
            .update_wallet_setting(UpdateWalletSettingRequest {
                private_enabled: Some(private_enabled),
                master_identity_public_key: None,
            })
            .await?;
        Ok(())
    }

    /// Starts leaf optimization in the background.
    pub fn start_leaf_optimization(&self) {
        self.leaf_optimizer.start();
    }

    /// Cancels the ongoing leaf optimization.
    ///
    /// This sets a cancellation flag that is checked between rounds.
    /// The current round will complete before stopping.
    pub async fn cancel_leaf_optimization(&self) -> Result<(), SparkWalletError> {
        self.leaf_optimizer.cancel().await?;
        Ok(())
    }

    /// Returns the current optimization progress snapshot.
    pub fn get_leaf_optimization_progress(&self) -> OptimizationProgress {
        self.leaf_optimizer.progress()
    }

    /// Optimizes token outputs by consolidating them when there are more than the configured threshold.
    /// Processes one token at a time. Token identifier can be provided, otherwise one is automatically selected.
    /// Only optimizes if the number of outputs is greater than the configured threshold.
    pub async fn optimize_token_outputs(
        &self,
        token_identifier: Option<&str>,
    ) -> Result<(), SparkWalletError> {
        self.token_service
            .optimize_token_outputs(
                token_identifier,
                self.config
                    .token_outputs_optimization_options
                    .min_outputs_threshold,
            )
            .await?;
        Ok(())
    }

    /// Selects leaves with automatic retry logic.
    ///
    /// This method implements a multi-level retry strategy:
    /// 1. Outer retry loop (up to SELECT_LEAVES_MAX_RETRIES times) handles general failures like
    ///    attempting to pay before initial sync by refreshing leaves between retries.
    /// 2. Inner retry logic handles insufficient funds when optimization is in progress by
    ///    cancelling the optimization and retrying once.
    async fn select_leaves_with_retry(
        &self,
        target_amounts: Option<&TargetAmounts>,
    ) -> Result<spark::tree::LeavesReservation, SparkWalletError> {
        use spark::tree::TreeServiceError;

        let mut reservation: Option<spark::tree::LeavesReservation> = None;

        for i in 0..SELECT_LEAVES_MAX_RETRIES {
            // Try to select leaves with inner optimization retry logic
            let reserve_result = self
                .try_select_leaves_with_optimization_retry(target_amounts)
                .await;

            match reserve_result {
                Ok(r) => {
                    reservation = Some(r);
                    break;
                }
                Err(e) => {
                    error!("Failed to select leaves: {e:?}");
                }
            }

            info!("Failed to select leaves, refreshing leaves and retrying");
            self.tree_service.refresh_leaves().await?;

            // Check if we have enough funds after refresh
            if let Some(target_amounts) = target_amounts {
                let available_balance = self.tree_service.get_available_balance().await?;
                if available_balance < target_amounts.total_sats() {
                    info!("Not enough funds to select leaves after refresh");
                    return Err(TreeServiceError::InsufficientFunds.into());
                }
            }

            // Sleep between retries (except on the last one)
            if i < SELECT_LEAVES_MAX_RETRIES - 1 {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }

        reservation.ok_or_else(|| {
            TreeServiceError::Generic("Failed to select leaves after all retries".to_string())
                .into()
        })
    }

    /// Inner helper that tries to select leaves and handles optimization cancellation.
    ///
    /// If select_leaves fails with InsufficientFunds and the optimizer has reserved
    /// leaves, this method cancels the optimization and retries once.
    async fn try_select_leaves_with_optimization_retry(
        &self,
        target_amounts: Option<&TargetAmounts>,
    ) -> Result<spark::tree::LeavesReservation, SparkWalletError> {
        use spark::tree::{ReservationPurpose, TreeServiceError};

        match self
            .tree_service
            .select_leaves(
                target_amounts,
                ReservationPurpose::Payment,
                SelectLeavesOptions::default(),
            )
            .await
        {
            Ok(reservation) => Ok(reservation),
            Err(TreeServiceError::InsufficientFunds) => {
                // Check if optimization has reserved leaves
                if self.leaf_optimizer.is_running() {
                    debug!(
                        "Insufficient funds with optimization in progress, cancelling optimization and retrying"
                    );

                    // Cancel optimization and wait for it to release leaves
                    if let Err(e) = self.leaf_optimizer.cancel().await {
                        debug!("Failed to cancel optimization: {:?}", e);
                    }

                    // Retry select_leaves
                    let reservation = self
                        .tree_service
                        .select_leaves(
                            target_amounts,
                            ReservationPurpose::Payment,
                            SelectLeavesOptions::default(),
                        )
                        .await?;
                    Ok(reservation)
                } else {
                    Err(TreeServiceError::InsufficientFunds.into())
                }
            }
            Err(e) => Err(e.into()),
        }
    }
}

async fn claim_pending_transfers(
    our_pubkey: PublicKey,
    transfer_service: &Arc<TransferService>,
    tree_service: &Arc<dyn TreeService>,
    htlc_service: &Arc<HtlcService>,
    ssp_client: &Arc<ServiceProvider>,
) -> Result<Vec<WalletTransfer>, SparkWalletError> {
    debug!("Claiming all pending transfers");
    let transfers = transfer_service
        .query_claimable_receiver_transfers(None)
        .await?;

    if transfers.is_empty() {
        debug!("No pending transfers found");
        return Ok(vec![]);
    }

    debug!(
        "Retrieved {} pending transfers, starting claims",
        transfers.len()
    );

    for (i, transfer) in transfers.items.iter().enumerate() {
        debug!("Claiming transfer: {}/{}", i + 1, transfers.len());
        claim_transfer(transfer, transfer_service, tree_service).await?;
        debug!(
            "Successfully claimed transfer: {}/{}",
            i + 1,
            transfers.len()
        );
    }

    let mut transfers = transfers;
    for transfer in transfers.items.iter_mut() {
        transfer.status = TransferStatus::Completed;
    }

    debug!("Claimed all transfers, creating wallet transfers");
    Ok(create_transfers(
        transfers,
        ssp_client,
        htlc_service,
        our_pubkey,
        ssp_client.identity_public_key(),
    )
    .await?
    .items)
}

async fn create_transfers(
    transfers: PagingResult<Transfer>,
    ssp_client: &Arc<ServiceProvider>,
    htlc_service: &Arc<HtlcService>,
    our_public_key: PublicKey,
    ssp_public_key: PublicKey,
) -> Result<PagingResult<WalletTransfer>, SparkWalletError> {
    let mut preimage_swap_transfer_ids = Vec::new();
    let mut transfer_ids = Vec::new();

    for transfer in &transfers.items {
        let transfer_id = transfer.id.to_string();
        if transfer.transfer_type == TransferType::PreimageSwap {
            preimage_swap_transfer_ids.push(transfer_id.clone());
        }
        transfer_ids.push(transfer_id);
    }

    let ssp_tranfers = ssp_client.get_transfers(transfer_ids.clone()).await?;
    let ssp_transfers_map: HashMap<String, SspTransfer> = ssp_tranfers
        .into_iter()
        .filter_map(|t| t.spark_id.clone().map(|spark_id| (spark_id, t.clone())))
        .collect();

    let htlc_requests = htlc_service
        .query_htlc(
            QueryHtlcFilter {
                transfer_ids: preimage_swap_transfer_ids,
                match_role: PreimageRequestRole::ReceiverAndSender,
                identity_public_key: our_public_key,
                status: None,
                payment_hashes: Vec::new(),
            },
            None,
        )
        .await?
        .items;

    let htlc_requests_map: HashMap<String, PreimageRequestWithTransfer> = htlc_requests
        .into_iter()
        .filter_map(|t| {
            t.transfer
                .clone()
                .map(|transfer| (transfer.id.to_string(), t))
        })
        .collect();

    Ok(transfers.map(|t| {
        WalletTransfer::from_transfer(
            t.clone(),
            ssp_transfers_map.get(&t.id.to_string()).cloned(),
            htlc_requests_map
                .get(&t.id.to_string())
                .cloned()
                .map(Into::into),
            our_public_key,
            ssp_public_key,
        )
    }))
}

async fn create_transfer(
    transfer: Transfer,
    ssp_client: &Arc<ServiceProvider>,
    htlc_service: &Arc<HtlcService>,
    our_public_key: PublicKey,
    ssp_public_key: PublicKey,
) -> Result<WalletTransfer, SparkWalletError> {
    let ssp_transfer = ssp_client
        .get_transfers(vec![transfer.id.to_string()])
        .await?
        .into_iter()
        .next();

    let preimage_request = if transfer.transfer_type == TransferType::PreimageSwap {
        htlc_service
            .query_htlc(
                QueryHtlcFilter {
                    transfer_ids: vec![transfer.id.to_string()],
                    match_role: PreimageRequestRole::ReceiverAndSender,
                    identity_public_key: our_public_key,
                    status: None,
                    payment_hashes: Vec::new(),
                },
                None,
            )
            .await?
            .items
            .first()
            .cloned()
    } else {
        None
    };

    Ok(WalletTransfer::from_transfer(
        transfer,
        ssp_transfer,
        preimage_request.map(Into::into),
        our_public_key,
        ssp_public_key,
    ))
}

async fn claim_transfer(
    transfer: &Transfer,
    transfer_service: &Arc<TransferService>,
    tree_service: &Arc<dyn TreeService>,
) -> Result<Vec<TreeNode>, SparkWalletError> {
    trace!("Claiming transfer with id: {}", transfer.id);
    let claimed_nodes = transfer_service.claim_transfer(transfer, None).await?;

    trace!("Inserting claimed leaves after claiming transfer");
    let result_nodes = tree_service.insert_leaves(claimed_nodes.clone()).await?;

    Ok(result_nodes)
}

/// Event handler that bridges OptimizationEvent to WalletEvent.
struct WalletOptimizationEventHandler {
    event_manager: Arc<EventManager>,
}

impl OptimizationEventHandler for WalletOptimizationEventHandler {
    fn on_optimization_event(&self, event: OptimizationEvent) {
        self.event_manager
            .notify_listeners(WalletEvent::Optimization(event));
    }
}

struct BackgroundProcessor {
    operator_pool: Arc<OperatorPool>,
    event_manager: Arc<EventManager>,
    identity_public_key: PublicKey,
    reconnect_interval: Duration,
    tree_service: Arc<dyn TreeService>,
    ssp_client: Arc<ServiceProvider>,
    transfer_service: Arc<TransferService>,
    htlc_service: Arc<HtlcService>,
    leaf_optimizer: Arc<LeafOptimizer>,
    auto_optimize_enabled: bool,
    token_service: Arc<TokenService>,
    token_outputs_optimization_options: TokenOutputsOptimizationOptions,
}

impl BackgroundProcessor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        operator_pool: Arc<OperatorPool>,
        event_manager: Arc<EventManager>,
        identity_public_key: PublicKey,
        reconnect_interval: Duration,
        tree_service: Arc<dyn TreeService>,
        ssp_client: Arc<ServiceProvider>,
        transfer_service: Arc<TransferService>,
        htlc_service: Arc<HtlcService>,
        leaf_optimizer: Arc<LeafOptimizer>,
        auto_optimize_enabled: bool,
        token_service: Arc<TokenService>,
        token_outputs_optimization_options: TokenOutputsOptimizationOptions,
    ) -> Self {
        Self {
            operator_pool,
            event_manager,
            identity_public_key,
            reconnect_interval,
            tree_service,
            ssp_client,
            transfer_service,
            htlc_service,
            leaf_optimizer,
            auto_optimize_enabled,
            token_service,
            token_outputs_optimization_options,
        }
    }

    fn maybe_start_optimization(&self) {
        if self.auto_optimize_enabled {
            self.leaf_optimizer.start();
        }
    }

    pub async fn run_background_tasks(self: &Arc<Self>, cancellation_token: watch::Receiver<()>) {
        let cloned_self = Arc::clone(self);
        tokio::spawn(async move {
            cloned_self
                .run_background_tasks_inner(cancellation_token)
                .await;
        });
    }

    async fn run_background_tasks_inner(self: &Arc<Self>, cancellation_token: watch::Receiver<()>) {
        let (event_tx, event_stream) = broadcast::channel(100);
        let operator_pool = Arc::clone(&self.operator_pool);
        let reconnect_interval = self.reconnect_interval;
        let identity_public_key = self.identity_public_key;
        let mut cancellation_token_for_events = cancellation_token.clone();
        tokio::spawn(async move {
            subscribe_server_events(
                identity_public_key,
                operator_pool,
                &event_tx,
                reconnect_interval,
                &mut cancellation_token_for_events,
            )
            .await;
        });

        if let Err(e) = self.tree_service.refresh_leaves().await {
            error!("Error refreshing leaves on startup: {:?}", e);
        }

        if let Err(e) = self.token_service.refresh_tokens_outputs().await {
            error!("Error refreshing token outputs on startup: {:?}", e);
        }

        // Start token output optimization background task if configured
        if let Some(interval) = self
            .token_outputs_optimization_options
            .auto_optimize_interval
        {
            let cloned_self = Arc::clone(self);
            let cancellation_token_clone = cancellation_token.clone();
            tokio::spawn(async move {
                cloned_self
                    .run_token_output_optimization(interval, cancellation_token_clone)
                    .await;
            });
        }

        self.process_events(event_stream).await;
    }

    async fn process_events(&self, mut event_stream: broadcast::Receiver<SparkEvent>) {
        use broadcast::error::RecvError;

        loop {
            match event_stream.recv().await {
                Ok(event) => {
                    debug!("Received event: {event}");
                    trace!("Received event: {event:?}");
                    let result = match event.clone() {
                        SparkEvent::Transfer(transfer) => {
                            self.process_transfer_event(*transfer).await
                        }
                        SparkEvent::Deposit(deposit) => self.process_deposit_event(*deposit).await,
                        SparkEvent::Connected => self.process_connected_event().await,
                        SparkEvent::Disconnected => self.process_disconnected_event().await,
                    };
                    debug!("Processed event: {event}");

                    if let Err(e) = result {
                        error!("Error processing event: {e:?}");
                    }
                }

                Err(RecvError::Lagged(skipped)) => {
                    error!("Event stream lagged, skipped {} messages", skipped);
                    continue;
                }
                Err(RecvError::Closed) => {
                    info!("Event stream closed, stopping event processing");
                    break;
                }
            }
        }
    }

    async fn process_deposit_event(&self, deposit: TreeNode) -> Result<(), SparkWalletError> {
        let id = deposit.id.clone();
        info!("Inserting deposit leaf: {:?}", deposit);
        self.tree_service.insert_leaves(vec![deposit]).await?;
        self.event_manager
            .notify_listeners(WalletEvent::DepositConfirmed(id));
        self.maybe_start_optimization();
        Ok(())
    }

    async fn process_transfer_event(&self, transfer: Transfer) -> Result<(), SparkWalletError> {
        info!("Processing transfer event: {:?}", transfer.transfer_type);
        // Skip claiming counter swap transfer as these are claimed synchronously by the Swap::swap_leaves() method.
        if transfer.transfer_type == spark::services::TransferType::CounterSwap
            || transfer.transfer_type == spark::services::TransferType::CounterSwapV3
        {
            debug!(
                "Received counter swap transfer, not claiming: {:?}",
                transfer
            );
            return Ok(());
        }

        // get the ssp transfer details, if it fails just use None
        // Internal transfers will not have an SSP entry so just skip it
        let ssp_transfer = if transfer.transfer_type == spark::services::TransferType::Transfer {
            None
        } else {
            self.ssp_client
                .get_transfers(vec![transfer.id.to_string()])
                .await
                .unwrap_or_default()
                .into_iter()
                .next()
        };

        let htlc = if transfer.transfer_type == spark::services::TransferType::PreimageSwap {
            self.htlc_service
                .query_htlc(
                    QueryHtlcFilter {
                        transfer_ids: vec![transfer.id.to_string()],
                        match_role: PreimageRequestRole::Receiver,
                        identity_public_key: self.identity_public_key,
                        payment_hashes: vec![],
                        status: None,
                    },
                    None,
                )
                .await?
                .items
                .first()
                .cloned()
                .map(Into::into)
        } else {
            None
        };

        self.event_manager
            .notify_listeners(WalletEvent::TransferClaimStarting(
                WalletTransfer::from_transfer(
                    transfer.clone(),
                    ssp_transfer.clone(),
                    htlc.clone(),
                    self.identity_public_key,
                    self.ssp_client.identity_public_key(),
                ),
            ));

        trace!("Claiming transfer from event");
        claim_transfer(&transfer, &self.transfer_service, &self.tree_service).await?;
        trace!("Claimed transfer from event");

        // Update transfer status before notifying listeners
        let mut claimed_transfer = transfer;
        claimed_transfer.status = TransferStatus::Completed;
        self.event_manager
            .notify_listeners(WalletEvent::TransferClaimed(WalletTransfer::from_transfer(
                claimed_transfer,
                ssp_transfer,
                htlc,
                self.identity_public_key,
                self.ssp_client.identity_public_key(),
            )));
        self.maybe_start_optimization();

        Ok(())
    }

    async fn process_connected_event(&self) -> Result<(), SparkWalletError> {
        self.event_manager
            .notify_listeners(WalletEvent::StreamConnected);

        match claim_pending_transfers(
            self.identity_public_key,
            &self.transfer_service,
            &self.tree_service,
            &self.htlc_service,
            &self.ssp_client,
        )
        .await
        {
            Ok(transfers) => {
                debug!(
                    "Claimed {} pending transfers on stream reconnection",
                    transfers.len()
                );
                if !transfers.is_empty() {
                    self.maybe_start_optimization();
                }
                for transfer in &transfers {
                    self.event_manager
                        .notify_listeners(WalletEvent::TransferClaimed(transfer.clone()));
                }
            }
            Err(e) => {
                debug!(
                    "Error claiming pending transfers on stream reconnection: {:?}",
                    e
                );
            }
        };
        self.event_manager.notify_listeners(WalletEvent::Synced);

        Ok(())
    }

    async fn process_disconnected_event(&self) -> Result<(), SparkWalletError> {
        self.event_manager
            .notify_listeners(WalletEvent::StreamDisconnected);
        Ok(())
    }

    async fn run_token_output_optimization(
        &self,
        interval: Duration,
        mut cancellation_token: watch::Receiver<()>,
    ) {
        info!(
            "Starting automatic token output optimization with interval: {:?}",
            interval
        );

        let run_optimization = || async {
            debug!("Running automatic token output optimization");
            match self
                .token_service
                .optimize_token_outputs(
                    None,
                    self.token_outputs_optimization_options
                        .min_outputs_threshold,
                )
                .await
            {
                Ok(_) => {
                    debug!("Automatic token output optimization completed successfully");
                }
                Err(e) => {
                    error!("Error during automatic token output optimization: {:?}", e);
                }
            }
        };

        // Run first optimization immediately
        run_optimization().await;

        loop {
            // Wait for the interval or cancellation
            tokio::select! {
                _ = tokio::time::sleep(interval) => {
                    run_optimization().await;
                }
                _ = cancellation_token.changed() => {
                    info!("Stopping automatic token output optimization");
                    break;
                }
            }
        }
    }
}
