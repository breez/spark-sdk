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
    operator::{Operator, OperatorPool, rpc::ConnectionManager},
    services::{
        CoopExitFeeQuote, CoopExitService, DepositService, ExitSpeed, LightningReceivePayment,
        LightningSendPayment, LightningService, Swap, TimelockManager, Transfer, TransferId,
        TransferService,
    },
    signer::Signer,
    ssp::ServiceProvider,
    tree::{LeavesReservation, TargetAmounts, TreeNode, TreeNodeId, TreeService, TreeState},
    utils::paging::PagingFilter,
};
use tokio::sync::{broadcast, watch};
use tracing::{debug, error, info, trace};

use crate::{
    leaf::WalletLeaf,
    model::{PayLightningInvoiceResult, WalletInfo, WalletTransfer},
};

use super::{SparkWalletConfig, SparkWalletError};

pub struct SparkWallet<S> {
    /// Cancellation token to stop background tasks. It is dropped when the wallet is dropped to stop background tasks.
    #[allow(dead_code)]
    cancel: watch::Sender<()>,
    config: SparkWalletConfig,
    deposit_service: DepositService<S>,
    identity_public_key: PublicKey,
    signer: S,
    swap_service: Arc<Swap<S>>,
    tree_service: Arc<TreeService<S>>,
    coop_exit_service: Arc<CoopExitService<S>>,
    transfer_service: Arc<TransferService<S>>,
    lightning_service: Arc<LightningService<S>>,
}

impl<S: Signer + Clone + Send + Sync + 'static> SparkWallet<S> {
    pub async fn connect(config: SparkWalletConfig, signer: S) -> Result<Self, SparkWalletError> {
        config.validate()?;
        let identity_public_key = signer.get_identity_public_key()?;
        let connection_manager = ConnectionManager::new();

        let bitcoin_service = BitcoinService::new(config.network);
        let service_provider = Arc::new(ServiceProvider::new(
            config.service_provider_config.clone(),
            signer.clone(),
        ));

        let operator_pool = Arc::new(
            OperatorPool::connect(
                &config.operator_pool,
                &connection_manager,
                Arc::new(signer.clone()),
            )
            .await?,
        );
        let lightning_service = Arc::new(LightningService::new(
            operator_pool.clone(),
            service_provider.clone(),
            config.network,
            signer.clone(),
            config.split_secret_threshold,
        ));
        let deposit_service = DepositService::new(
            bitcoin_service,
            identity_public_key,
            config.network,
            operator_pool.clone(),
            signer.clone(),
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
        let tree_service = Arc::new(TreeService::new(
            identity_public_key,
            config.network,
            operator_pool.clone(),
            tree_state,
            Arc::clone(&timelock_manager),
            signer.clone(),
        ));

        let coop_exit_service = Arc::new(CoopExitService::new(
            operator_pool.clone(),
            service_provider.clone(),
            Arc::clone(&transfer_service),
            config.network,
            signer.clone(),
        ));
        let swap_service = Arc::new(Swap::new(
            config.network,
            operator_pool.clone(),
            signer.clone(),
            Arc::clone(&service_provider),
            Arc::clone(&transfer_service),
        ));

        let (cancel, cancellation_token) = watch::channel(());
        let coordinator = operator_pool.get_coordinator().clone();
        let reconnect_interval = Duration::from_secs(config.reconnect_interval_seconds);
        let background_processor = Arc::new(BackgroundProcessor::new(
            coordinator,
            identity_public_key,
            reconnect_interval,
            Arc::clone(&transfer_service),
            Arc::clone(&tree_service),
        ));
        background_processor
            .run_background_tasks(cancellation_token.clone())
            .await;

        Ok(Self {
            cancel,
            config,
            deposit_service,
            identity_public_key,
            signer,
            swap_service,
            tree_service,
            coop_exit_service,
            transfer_service,
            lightning_service,
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
        let leaves_reservation = self.select_leaves(Some(&target_amounts)).await?;
        // start the lightning swap with the operator
        let swap = with_reserved_leaves(
            self.tree_service.clone(),
            async {
                Ok(self
                    .lightning_service
                    .start_lightning_swap(invoice, amount_to_send, &leaves_reservation.leaves)
                    .await?)
            },
            &leaves_reservation,
        )
        .await?;

        // send the leaves to the operator
        let _ = self
            .transfer_service
            .deliver_transfer_package(&swap.transfer, &swap.leaves, HashMap::new())
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
        let reservation = self.select_leaves(target_amounts.as_ref()).await?;

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
        // TODO: update local tree here, otherwise a failure in collect_leaves will result in out of date state.
        let collected_leaves = self.tree_service.collect_leaves(deposit_nodes).await?;
        debug!("Collected deposit leaves: {:?}", collected_leaves);
        Ok(collected_leaves.into_iter().map(WalletLeaf::from).collect())
    }

    pub async fn generate_deposit_address(
        &self,
        is_static: bool,
    ) -> Result<Address, SparkWalletError> {
        let leaf_id = TreeNodeId::generate();
        let signing_public_key = self.signer.get_public_key_for_node(&leaf_id)?;
        let address = self
            .deposit_service
            .generate_deposit_address(signing_public_key, &leaf_id, is_static)
            .await?;

        // TODO: Watch this address for deposits.

        Ok(address.address)
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

    async fn swap_leaves_internal(
        &self,
        leaves: &[TreeNode],
        target_amounts: Option<&TargetAmounts>,
    ) -> Result<Vec<TreeNode>, SparkWalletError> {
        let target_amounts = target_amounts.map(|ta| ta.to_vec()).unwrap_or_default();
        let transfer = self
            .swap_service
            .swap_leaves(leaves, target_amounts)
            .await?;
        let leaves = claim_transfer(&transfer, &self.transfer_service, &self.tree_service).await?;
        Ok(leaves)
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
        let leaves_reservation = self.select_leaves(Some(&target_amounts)).await?;

        let transfer = with_reserved_leaves(
            self.tree_service.clone(),
            async {
                Ok(self
                    .transfer_service
                    .transfer_leaves_to(leaves_reservation.leaves.clone(), &receiver_pubkey)
                    .await?)
            },
            &leaves_reservation,
        )
        .await?;

        Ok(transfer.into())
    }

    /// Claims all pending transfers.
    pub async fn claim_pending_transfers(&self) -> Result<Vec<WalletTransfer>, SparkWalletError> {
        claim_pending_transfers(&self.transfer_service, &self.tree_service).await
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
        ))
    }

    pub async fn get_balance(&self) -> Result<u64, SparkWalletError> {
        Ok(self.tree_service.get_available_balance().await?)
    }

    pub async fn list_transfers(
        &self,
        paging: Option<PagingFilter>,
    ) -> Result<Vec<WalletTransfer>, SparkWalletError> {
        let transfers = self.transfer_service.query_transfers(paging).await?;
        Ok(transfers.into_iter().map(WalletTransfer::from).collect())
    }

    pub async fn list_pending_transfers(
        &self,
        paging: Option<PagingFilter>,
    ) -> Result<Vec<WalletTransfer>, SparkWalletError> {
        let transfers = self
            .transfer_service
            .query_pending_transfers(paging)
            .await?;
        Ok(transfers.into_iter().map(WalletTransfer::from).collect())
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

    /// Selects leaves from the tree that sum up to exactly the target amounts.
    /// If such a combination of leaves does not exist, it performs a swap to get a set of leaves matching the target amounts.
    /// If no leaves can be selected, returns an error
    async fn select_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
    ) -> Result<LeavesReservation, SparkWalletError> {
        trace!("Selecting leaves for target amounts: {target_amounts:?}");
        let reservation = self
            .tree_service
            .reserve_leaves(target_amounts, false)
            .await?;
        let Some(reservation) = reservation else {
            return Err(SparkWalletError::InsufficientFunds);
        };

        trace!(
            "Selected leaves got reservation: {:?} ({})",
            reservation.id,
            reservation.sum()
        );

        // Handle cases where no swapping is needed:
        // - The target amount is zero
        // - The reservation already matches the total target amounts and each target amount
        //   can be selected from the reserved leaves
        let total_amount_sats = target_amounts.map(|ta| ta.total_sats()).unwrap_or(0);
        if (total_amount_sats == 0 || reservation.sum() == total_amount_sats)
            && self
                .tree_service
                .select_leaves_by_amounts(&reservation.leaves, target_amounts)
                .is_ok()
        {
            trace!("Selected leaves match requirements, no swap needed");
            return Ok(reservation);
        }

        // Swap the leaves to match the target amount.
        with_reserved_leaves(
            self.tree_service.clone(),
            self.swap_leaves_internal(&reservation.leaves, target_amounts),
            &reservation,
        )
        .await?;
        trace!("Swapped leaves to match target amount");
        // Now the leaves should contain the exact amount.
        let reservation = self
            .tree_service
            .reserve_leaves(target_amounts, true)
            .await?
            .ok_or(SparkWalletError::InsufficientFunds)?;
        trace!(
            "Selected leaves got reservation after swap: {:?} ({})",
            reservation.id,
            reservation.sum()
        );
        Ok(reservation)
    }

    pub async fn sync(&self) -> Result<(), SparkWalletError> {
        self.tree_service.refresh_leaves().await?;
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
        let leaves_reservation = self.select_leaves(target_amounts.as_ref()).await?;

        let transfer = with_reserved_leaves(
            self.tree_service.clone(),
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

        Ok(transfer.into())
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
}

async fn with_reserved_leaves<F, R, S>(
    tree_service: Arc<TreeService<S>>,
    f: F,
    leaves: &LeavesReservation,
) -> Result<R, SparkWalletError>
where
    F: Future<Output = Result<R, SparkWalletError>>,
    S: Signer,
{
    match f.await {
        Ok(r) => {
            tree_service.finalize_reservation(leaves.id.clone()).await;
            Ok(r)
        }
        Err(e) => {
            tree_service.cancel_reservation(leaves.id.clone()).await;
            Err(e)
        }
    }
}

async fn claim_pending_transfers<S: Signer>(
    transfer_service: &Arc<TransferService<S>>,
    tree_service: &Arc<TreeService<S>>,
) -> Result<Vec<WalletTransfer>, SparkWalletError> {
    trace!("Claiming all pending transfers");
    let transfers = transfer_service
        .query_pending_receiver_transfers(None)
        .await?;
    trace!("There are {} pending transfers", transfers.len());
    for transfer in &transfers {
        claim_transfer(transfer, transfer_service, tree_service).await?;
    }

    Ok(transfers.into_iter().map(WalletTransfer::from).collect())
}

async fn claim_transfer<S: Signer>(
    transfer: &Transfer,
    transfer_service: &Arc<TransferService<S>>,
    tree_service: &Arc<TreeService<S>>,
) -> Result<Vec<TreeNode>, SparkWalletError> {
    trace!("Claiming transfer with id: {}", transfer.id);
    let claimed_nodes = transfer_service.claim_transfer(transfer, None).await?;

    trace!("Inserting claimed leaves after claiming transfer");
    let result_nodes = tree_service.insert_leaves(claimed_nodes.clone()).await?;

    // TODO: Emit events if emit is true
    // TODO: Optimize leaves if optimize is true and the transfer type is not counter swap

    Ok(result_nodes)
}

struct BackgroundProcessor<S: Signer> {
    coordinator: Operator<S>,
    identity_public_key: PublicKey,
    reconnect_interval: Duration,
    transfer_service: Arc<TransferService<S>>,
    tree_service: Arc<TreeService<S>>,
}

impl<S> BackgroundProcessor<S>
where
    S: Signer + Clone + Send + Sync + 'static,
{
    pub fn new(
        coordinator: Operator<S>,
        identity_public_key: PublicKey,
        reconnect_interval: Duration,
        transfer_service: Arc<TransferService<S>>,
        tree_service: Arc<TreeService<S>>,
    ) -> Self {
        Self {
            coordinator,
            identity_public_key,
            reconnect_interval,
            transfer_service,
            tree_service,
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
        let coordinator = self.coordinator.clone();
        let reconnect_interval = self.reconnect_interval;
        let identity_public_key = self.identity_public_key;
        tokio::spawn(async move {
            subscribe_server_events(
                identity_public_key,
                &coordinator,
                &event_tx,
                reconnect_interval,
                &mut cancellation_token,
            )
            .await;
        });

        let ignore_transfers =
            match claim_pending_transfers(&self.transfer_service, &self.tree_service).await {
                Ok(transfers) => {
                    debug!("Claimed {} pending transfers on startup", transfers.len());
                    transfers.into_iter().map(|t| t.id).collect()
                }
                Err(e) => {
                    debug!("Error claiming pending transfers on startup: {:?}", e);
                    HashSet::new()
                }
            };

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
            };

            if let Err(e) = result {
                error!("Error processing event: {:?}", e);
            }
        }

        info!("Event stream closed, stopping event processing");
    }

    async fn process_deposit_event(&self, deposit: TreeNode) -> Result<(), SparkWalletError> {
        self.tree_service
            .insert_leaves(vec![deposit.clone()])
            .await?;
        self.tree_service.collect_leaves(vec![deposit]).await?;
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
        Ok(())
    }
}
