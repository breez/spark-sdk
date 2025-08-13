use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use bitcoin::{
    Address, Transaction,
    address::NetworkUnchecked,
    key::Secp256k1,
    secp256k1::{PublicKey, ecdsa::Signature},
};

use spark::{
    address::SparkAddress,
    bitcoin::BitcoinService,
    events::{SparkEvent, subscribe_server_events},
    operator::{OperatorPool, rpc::ConnectionManager},
    services::{
        CoopExitFeeQuote, CoopExitService, DepositService, ExitSpeed, LightningReceivePayment,
        LightningSendPayment, LightningService, QueryTokenTransactionsFilter, StaticDepositQuote,
        Swap, TimelockManager, TokenService, TokenTransaction, Transfer, TransferId,
        TransferService, TransferTokenOutput, Utxo,
    },
    signer::Signer,
    ssp::{ServiceProvider, SspTransfer},
    tree::{LeavesReservation, TargetAmounts, TreeNode, TreeNodeId, TreeService, TreeState},
    utils::paging::PagingFilter,
};
use tokio::sync::{broadcast, watch};
use tokio_with_wasm::alias as tokio;
use tracing::{debug, error, info, trace};

use crate::{
    ListTokenTransactionsRequest, TokenBalance, WalletEvent,
    event::EventManager,
    model::{PayLightningInvoiceResult, WalletInfo, WalletLeaf, WalletTransfer},
};

use super::{SparkWalletConfig, SparkWalletError};

pub struct SparkWallet<S> {
    /// Cancellation token to stop background tasks. It is dropped when the wallet is dropped to stop background tasks.
    #[allow(dead_code)]
    cancel: watch::Sender<()>,
    config: SparkWalletConfig,
    deposit_service: DepositService<S>,
    event_manager: Arc<EventManager>,
    identity_public_key: PublicKey,
    signer: Arc<S>,
    tree_service: Arc<TreeService<S>>,
    coop_exit_service: Arc<CoopExitService<S>>,
    transfer_service: Arc<TransferService<S>>,
    lightning_service: Arc<LightningService<S>>,
    ssp_client: Arc<ServiceProvider<S>>,
    token_service: Arc<TokenService<S>>,
}

impl<S: Signer> SparkWallet<S> {
    pub async fn connect(config: SparkWalletConfig, signer: S) -> Result<Self, SparkWalletError> {
        config.validate()?;
        let identity_public_key = signer.get_identity_public_key()?;
        let connection_manager = ConnectionManager::new();

        let signer = Arc::new(signer);

        let bitcoin_service = BitcoinService::new(config.network);
        let service_provider = Arc::new(ServiceProvider::new(
            config.service_provider_config.clone(),
            signer.clone(),
        ));

        let operator_pool = Arc::new(
            OperatorPool::connect(
                &config.operator_pool,
                &connection_manager,
                Arc::clone(&signer),
            )
            .await?,
        );
        let lightning_service = Arc::new(LightningService::new(
            operator_pool.clone(),
            service_provider.clone(),
            config.network,
            Arc::clone(&signer),
            config.split_secret_threshold,
        ));
        let deposit_service = DepositService::new(
            bitcoin_service,
            identity_public_key,
            config.network,
            operator_pool.clone(),
            service_provider.clone(),
            Arc::clone(&signer),
        );

        let transfer_service = Arc::new(TransferService::new(
            signer.clone(),
            config.network,
            config.split_secret_threshold,
            operator_pool.clone(),
        ));

        let timelock_manager = Arc::new(TimelockManager::new(
            signer.clone(),
            config.network,
            operator_pool.clone(),
            Arc::clone(&transfer_service),
        ));

        let tree_state = TreeState::new();

        let coop_exit_service = Arc::new(CoopExitService::new(
            operator_pool.clone(),
            service_provider.clone(),
            Arc::clone(&transfer_service),
            config.network,
            Arc::clone(&signer),
        ));

        let swap_service = Swap::new(
            config.network,
            operator_pool.clone(),
            Arc::clone(&signer),
            Arc::clone(&service_provider),
            Arc::clone(&transfer_service),
        );

        let tree_service = Arc::new(TreeService::new(
            identity_public_key,
            config.network,
            operator_pool.clone(),
            tree_state,
            Arc::clone(&timelock_manager),
            Arc::clone(&signer),
            swap_service,
        ));

        let token_service = Arc::new(TokenService::new(
            Arc::clone(&signer),
            operator_pool.clone(),
            config.network,
            config.split_secret_threshold,
            config.tokens_config.clone(),
        ));

        let event_manager = Arc::new(EventManager::new());
        let (cancel, cancellation_token) = watch::channel(());
        let reconnect_interval = Duration::from_secs(config.reconnect_interval_seconds);
        let background_processor = Arc::new(BackgroundProcessor::new(
            Arc::clone(&operator_pool),
            Arc::clone(&event_manager),
            identity_public_key,
            reconnect_interval,
            Arc::clone(&transfer_service),
            Arc::clone(&tree_service),
            Arc::clone(&service_provider),
        ));
        background_processor
            .run_background_tasks(cancellation_token.clone())
            .await;

        Ok(Self {
            cancel,
            config,
            deposit_service,
            event_manager,
            identity_public_key,
            signer,
            tree_service,
            coop_exit_service,
            transfer_service,
            lightning_service,
            ssp_client: service_provider.clone(),
            token_service,
        })
    }
}

impl<S: Signer> SparkWallet<S> {
    pub async fn list_leaves(&self) -> Result<Vec<WalletLeaf>, SparkWalletError> {
        let leaves = self.tree_service.list_leaves().await?;
        Ok(leaves.into_iter().map(WalletLeaf::from).collect())
    }

    pub async fn pay_lightning_invoice(
        &self,
        invoice: &str,
        amount_to_send: Option<u64>,
        max_fee_sat: Option<u64>,
        prefer_spark: bool,
    ) -> Result<PayLightningInvoiceResult, SparkWalletError> {
        let (total_amount_sat, receiver_spark_address) = self
            .lightning_service
            .validate_payment(invoice, max_fee_sat, amount_to_send, prefer_spark)
            .await?;

        if let Some(receiver_spark_address) = receiver_spark_address {
            return Ok(PayLightningInvoiceResult::Transfer(
                self.transfer(total_amount_sat, &receiver_spark_address)
                    .await?,
            ));
        }

        let target_amounts = TargetAmounts::new(total_amount_sat, None);
        let leaves_reservation = self
            .tree_service
            .select_leaves(Some(&target_amounts))
            .await?;
        // start the lightning swap with the operator
        let swap = self
            .tree_service
            .with_reserved_leaves(
                self.lightning_service.start_lightning_swap(
                    invoice,
                    amount_to_send,
                    &leaves_reservation.leaves,
                ),
                &leaves_reservation,
            )
            .await?;

        // send the leaves to the operator
        let _ = self
            .transfer_service
            .deliver_transfer_package(&swap.transfer, &swap.leaves, Default::default())
            .await?;

        // finalize the lightning swap with the ssp - send the actual lightning payment
        Ok(PayLightningInvoiceResult::LightningPayment(
            self.lightning_service
                .finalize_lightning_swap(&swap)
                .await?,
        ))
    }

    pub async fn create_lightning_invoice(
        &self,
        amount_sat: u64,
        description: Option<String>,
    ) -> Result<LightningReceivePayment, SparkWalletError> {
        Ok(self
            .lightning_service
            .create_lightning_invoice(amount_sat, description, None, None)
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
        let target_amounts = amount_sats.map(|amount| TargetAmounts::new(amount, None));
        let reservation = self
            .tree_service
            .select_leaves(target_amounts.as_ref())
            .await?;

        // Fetches fee quote for the coop exit then cancels the reservation.
        let fee_quote_res = self
            .coop_exit_service
            .fetch_coop_exit_fee_quote(reservation.leaves, withdrawal_address)
            .await;
        self.tree_service.cancel_reservation(reservation.id).await;

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
        // TODO: This entire function happens inside a txid mutex in the js sdk. It seems unnecessary here?

        let deposit_nodes = self.deposit_service.claim_deposit(tx, vout).await?;
        debug!("Claimed deposit root node: {:?}", deposit_nodes);
        let collected_leaves = self.tree_service.collect_leaves(deposit_nodes).await?;
        debug!("Collected deposit leaves: {:?}", collected_leaves);
        Ok(collected_leaves.into_iter().map(WalletLeaf::from).collect())
    }

    pub async fn claim_static_deposit(
        &self,
        quote: StaticDepositQuote,
    ) -> Result<WalletTransfer, SparkWalletError> {
        let transfer = self.deposit_service.claim_static_deposit(quote).await?;

        Ok(WalletTransfer::from_transfer(
            transfer,
            None,
            self.identity_public_key,
        ))
    }

    pub async fn refund_static_deposit(
        &self,
        tx: Transaction,
        output_index: Option<u32>,
        refund_address: &str,
        fee_sats: u64,
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
            .refund_static_deposit(tx, output_index, refund_address, fee_sats)
            .await?;

        Ok(refund_tx)
    }

    pub async fn generate_deposit_address(
        &self,
        is_static: bool,
    ) -> Result<Address, SparkWalletError> {
        let leaf_id = TreeNodeId::generate();
        let signing_public_key = if is_static {
            self.signer.get_static_deposit_public_key(0)?
        } else {
            self.signer.get_public_key_for_node(&leaf_id)?
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
    ) -> Result<Vec<Address>, SparkWalletError> {
        let static_addresses = self
            .deposit_service
            .query_static_deposit_addresses(paging)
            .await?;
        Ok(static_addresses
            .into_iter()
            .map(|addr| addr.address)
            .collect())
    }

    pub async fn list_unused_deposit_addresses(
        &self,
        paging: Option<PagingFilter>,
    ) -> Result<Vec<Address>, SparkWalletError> {
        let deposit_addresses = self
            .deposit_service
            .query_unused_deposit_addresses(paging)
            .await?;
        Ok(deposit_addresses
            .into_iter()
            .map(|addr| addr.address)
            .collect())
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
    ) -> Result<WalletTransfer, SparkWalletError> {
        // validate receiver address and get its pubkey
        if self.config.network != receiver_address.network {
            return Err(SparkWalletError::InvalidNetwork);
        }
        let receiver_pubkey = receiver_address.identity_public_key;

        // get leaves to transfer
        let target_amounts = TargetAmounts::new(amount_sat, None);
        let leaves_reservation = self
            .tree_service
            .select_leaves(Some(&target_amounts))
            .await?;

        let transfer = self
            .tree_service
            .with_reserved_leaves(
                self.transfer_service
                    .transfer_leaves_to(leaves_reservation.leaves.clone(), &receiver_pubkey),
                &leaves_reservation,
            )
            .await?;

        Ok(WalletTransfer::from_transfer(
            transfer,
            None,
            self.identity_public_key,
        ))
    }

    /// Claims all pending transfers.
    pub async fn claim_pending_transfers(&self) -> Result<Vec<WalletTransfer>, SparkWalletError> {
        claim_pending_transfers(
            self.identity_public_key,
            &self.transfer_service,
            &self.tree_service,
            &self.ssp_client,
        )
        .await
    }

    pub fn get_info(&self) -> WalletInfo {
        WalletInfo {
            identity_public_key: self.identity_public_key,
            network: self.config.network,
        }
    }

    pub async fn get_spark_address(&self) -> Result<SparkAddress, SparkWalletError> {
        Ok(SparkAddress::new(
            self.identity_public_key,
            self.config.network,
            None,
            None,
        ))
    }

    pub async fn get_balance(&self) -> Result<u64, SparkWalletError> {
        Ok(self.tree_service.get_available_balance().await?)
    }

    pub async fn list_transfers(
        &self,
        paging: Option<PagingFilter>,
    ) -> Result<Vec<WalletTransfer>, SparkWalletError> {
        let our_pubkey = self.identity_public_key;
        let transfers = self.transfer_service.query_transfers(paging).await?;
        create_transfers(transfers, &self.ssp_client, our_pubkey).await
    }

    pub async fn list_pending_transfers(
        &self,
        paging: Option<PagingFilter>,
    ) -> Result<Vec<WalletTransfer>, SparkWalletError> {
        let our_pubkey = self.identity_public_key;
        let transfers = self
            .transfer_service
            .query_pending_transfers(paging)
            .await?;
        create_transfers(transfers, &self.ssp_client, our_pubkey).await
    }

    /// Signs a message with the identity key using ECDSA and returns the signature.
    ///
    /// If exposing this, consider adding a prefix to prevent mistakenly signing messages.
    pub async fn sign_message(&self, message: &str) -> Result<Signature, SparkWalletError> {
        Ok(self.signer.sign_message_ecdsa_with_identity_key(message)?)
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
        self.token_service.refresh_tokens().await?;
        Ok(())
    }

    pub async fn withdraw(
        &self,
        withdrawal_address: &str,
        amount_sats: Option<u64>,
        exit_speed: ExitSpeed,
        fee_quote: CoopExitFeeQuote,
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

        // Select leaves for the withdrawal
        let target_amounts =
            amount_sats.map(|amount_sats| TargetAmounts::new(amount_sats, Some(fee_sats)));
        let leaves_reservation = self
            .tree_service
            .select_leaves(target_amounts.as_ref())
            .await?;

        let transfer = self
            .tree_service
            .with_reserved_leaves(
                self.withdraw_inner(
                    withdrawal_address,
                    exit_speed,
                    &leaves_reservation,
                    target_amounts.as_ref(),
                    fee_sats,
                    fee_quote.id,
                ),
                &leaves_reservation,
            )
            .await?;

        create_transfers(vec![transfer], &self.ssp_client, self.identity_public_key)
            .await?
            .first()
            .cloned()
            .ok_or(SparkWalletError::Generic(
                "Failed to create transfer".to_string(),
            ))
    }

    async fn withdraw_inner(
        &self,
        address: Address,
        exit_speed: ExitSpeed,
        leaves_reservation: &LeavesReservation,
        target_amounts: Option<&TargetAmounts>,
        fee_sats: u64,
        fee_quote_id: String,
    ) -> Result<Transfer, SparkWalletError> {
        let withdraw_all = target_amounts.is_none();
        let (withdraw_leaves, fee_leaves, fee_quote_id) = if withdraw_all {
            (leaves_reservation.leaves.clone(), None, None)
        } else {
            let target_leaves = self
                .tree_service
                .select_leaves_by_amounts(&leaves_reservation.leaves, target_amounts)?;
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
            return Err(SparkWalletError::InsufficientFunds);
        }

        // Perform the cooperative exit with the SSP
        let transfer = self
            .coop_exit_service
            .coop_exit(
                withdraw_leaves,
                &address,
                withdraw_all,
                exit_speed,
                fee_quote_id,
                fee_leaves,
            )
            .await?;

        Ok(transfer)
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
        let tokens_outputs = self.token_service.get_tokens_outputs().await;

        let balances = tokens_outputs
            .iter()
            .map(|(token_id, token_outputs)| {
                let balance = token_outputs
                    .outputs
                    .iter()
                    .map(|output| output.output.token_amount)
                    .sum();
                (
                    token_id.clone(),
                    TokenBalance {
                        balance,
                        token_metadata: token_outputs.metadata.clone(),
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
    ) -> Result<String, SparkWalletError> {
        let tx_hash = self.token_service.transfer_tokens(outputs).await?;
        Ok(tx_hash)
    }

    pub async fn list_token_transactions(
        &self,
        request: ListTokenTransactionsRequest,
    ) -> Result<Vec<TokenTransaction>, SparkWalletError> {
        self.token_service
            .query_token_transactions(
                QueryTokenTransactionsFilter {
                    owner_public_keys: request.owner_public_keys,
                    issuer_public_keys: request.issuer_public_keys,
                    token_transaction_hashes: request.token_transaction_hashes,
                    token_ids: request.token_ids,
                    output_ids: request.output_ids,
                },
                request.paging,
            )
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
}

async fn claim_pending_transfers<S: Signer>(
    our_pubkey: PublicKey,
    transfer_service: &Arc<TransferService<S>>,
    tree_service: &Arc<TreeService<S>>,
    ssp_client: &Arc<ServiceProvider<S>>,
) -> Result<Vec<WalletTransfer>, SparkWalletError> {
    trace!("Claiming all pending transfers");
    let transfers = transfer_service
        .query_pending_receiver_transfers(None)
        .await?;
    trace!("There are {} pending transfers", transfers.len());
    for transfer in &transfers {
        claim_transfer(transfer, transfer_service, tree_service).await?;
    }
    create_transfers(transfers, ssp_client, our_pubkey).await
}

async fn create_transfers<S: Signer>(
    transfers: Vec<Transfer>,
    ssp_client: &Arc<ServiceProvider<S>>,
    our_public_key: PublicKey,
) -> Result<Vec<WalletTransfer>, SparkWalletError> {
    let transfer_ids = transfers.iter().map(|t| t.id.to_string()).collect();
    let ssp_tranfers = ssp_client.get_transfers(transfer_ids).await?;
    let ssp_transfers_map: HashMap<String, SspTransfer> = ssp_tranfers
        .into_iter()
        .filter_map(|t| t.spark_id.clone().map(|spark_id| (spark_id, t.clone())))
        .collect();
    Ok(transfers
        .into_iter()
        .map(|t| {
            WalletTransfer::from_transfer(
                t.clone(),
                ssp_transfers_map.get(&t.id.to_string()).cloned(),
                our_public_key,
            )
        })
        .collect())
}

async fn claim_transfer<S: Signer>(
    transfer: &Transfer,
    transfer_service: &Arc<TransferService<S>>,
    tree_service: &Arc<TreeService<S>>,
) -> Result<Vec<TreeNode>, SparkWalletError> {
    trace!("Claiming transfer with id: {}", transfer.id);
    let claimed_nodes = transfer_service.claim_transfer(transfer, None).await?;

    trace!("Inserting claimed leaves after claiming transfer");
    let result_nodes = tree_service
        .insert_leaves(claimed_nodes.clone(), true)
        .await?;

    Ok(result_nodes)
}

struct BackgroundProcessor<S: Signer> {
    operator_pool: Arc<OperatorPool<S>>,
    event_manager: Arc<EventManager>,
    identity_public_key: PublicKey,
    reconnect_interval: Duration,
    transfer_service: Arc<TransferService<S>>,
    tree_service: Arc<TreeService<S>>,
    ssp_client: Arc<ServiceProvider<S>>,
}

impl<S: Signer> BackgroundProcessor<S> {
    pub fn new(
        operator_pool: Arc<OperatorPool<S>>,
        event_manager: Arc<EventManager>,
        identity_public_key: PublicKey,
        reconnect_interval: Duration,
        transfer_service: Arc<TransferService<S>>,
        tree_service: Arc<TreeService<S>>,
        ssp_client: Arc<ServiceProvider<S>>,
    ) -> Self {
        Self {
            operator_pool,
            event_manager,
            identity_public_key,
            reconnect_interval,
            transfer_service,
            tree_service,
            ssp_client,
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

    async fn run_background_tasks_inner(
        self: &Arc<Self>,
        mut cancellation_token: watch::Receiver<()>,
    ) {
        let (event_tx, event_stream) = broadcast::channel(100);
        let operator_pool = Arc::clone(&self.operator_pool);
        let reconnect_interval = self.reconnect_interval;
        let identity_public_key = self.identity_public_key;
        tokio::spawn(async move {
            subscribe_server_events(
                identity_public_key,
                operator_pool,
                &event_tx,
                reconnect_interval,
                &mut cancellation_token,
            )
            .await;
        });

        if let Err(e) = self.tree_service.refresh_leaves().await {
            error!("Error refreshing leaves on startup: {:?}", e);
        }

        let ignore_transfers = match claim_pending_transfers(
            self.identity_public_key,
            &self.transfer_service,
            &self.tree_service,
            &self.ssp_client,
        )
        .await
        {
            Ok(transfers) => {
                debug!("Claimed {} pending transfers on startup", transfers.len());
                for transfer in &transfers {
                    self.event_manager
                        .notify_listeners(WalletEvent::TransferClaimed(transfer.id.clone()));
                }
                transfers.into_iter().map(|t| t.id).collect()
            }
            Err(e) => {
                debug!("Error claiming pending transfers on startup: {:?}", e);
                HashSet::new()
            }
        };

        self.event_manager.notify_listeners(WalletEvent::Synced);
        self.process_events(event_stream, ignore_transfers).await;
    }

    async fn process_events(
        &self,
        mut event_stream: broadcast::Receiver<SparkEvent>,
        ignore_transfers: HashSet<TransferId>,
    ) {
        while let Ok(event) = event_stream.recv().await {
            debug!("Received event: {:?}", event);
            let result = match event {
                SparkEvent::Transfer(transfer) => {
                    if ignore_transfers.contains(&transfer.id) {
                        debug!("Ignoring transfer event: {:?}", transfer);
                        continue;
                    }

                    self.process_transfer_event(*transfer).await
                }
                SparkEvent::Deposit(deposit) => self.process_deposit_event(*deposit).await,
                SparkEvent::Connected => self.process_connected_event().await,
                SparkEvent::Disconnected => self.process_disconnected_event().await,
            };

            if let Err(e) = result {
                error!("Error processing event: {:?}", e);
            }
        }

        info!("Event stream closed, stopping event processing");
    }

    async fn process_deposit_event(&self, deposit: TreeNode) -> Result<(), SparkWalletError> {
        let id = deposit.id.clone();
        self.tree_service.collect_leaves(vec![deposit]).await?;
        self.event_manager
            .notify_listeners(WalletEvent::DepositConfirmed(id));
        Ok(())
    }

    async fn process_transfer_event(&self, transfer: Transfer) -> Result<(), SparkWalletError> {
        if transfer.transfer_type == spark::services::TransferType::CounterSwap {
            debug!(
                "Received counter swap transfer, not claiming: {:?}",
                transfer
            );
            return Ok(());
        }

        claim_transfer(&transfer, &self.transfer_service, &self.tree_service).await?;
        self.event_manager
            .notify_listeners(WalletEvent::TransferClaimed(transfer.id));
        Ok(())
    }

    async fn process_connected_event(&self) -> Result<(), SparkWalletError> {
        self.event_manager
            .notify_listeners(WalletEvent::StreamConnected);
        Ok(())
    }

    async fn process_disconnected_event(&self) -> Result<(), SparkWalletError> {
        self.event_manager
            .notify_listeners(WalletEvent::StreamDisconnected);
        Ok(())
    }
}
