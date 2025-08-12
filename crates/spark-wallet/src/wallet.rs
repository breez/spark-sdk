use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use bitcoin::{
    Address, Amount, CompressedPublicKey, OutPoint, Transaction, TxIn, TxOut,
    absolute::LockTime,
    address::NetworkUnchecked,
    key::Secp256k1,
    psbt,
    secp256k1::{PublicKey, ecdsa::Signature},
    transaction::Version,
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
    FeeBumpUtxo, LeafTxFeeBumpPsbts, ListTokenTransactionsRequest, TokenBalance, TxFeeBumpPsbt,
    WalletEvent,
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

    pub async fn unilateral_exit<F>(
        &self,
        fee_rate: u64,
        leaf_ids: Vec<TreeNodeId>,
        mut utxos: Vec<FeeBumpUtxo>,
        get_transaction_fn: impl Fn(String) -> F,
    ) -> Result<Vec<LeafTxFeeBumpPsbts>, SparkWalletError>
    where
        F: std::future::Future<Output = Result<Transaction, SparkWalletError>>,
    {
        if leaf_ids.is_empty() {
            return Err(SparkWalletError::ValidationError(
                "At least one leaf ID is required".to_string(),
            ));
        }
        if utxos.is_empty() {
            return Err(SparkWalletError::ValidationError(
                "At least one UTXO is required".to_string(),
            ));
        }

        let mut all_leaf_tx_fee_bump_psbts = Vec::new();
        let mut checked_txs = HashSet::new();

        // Fetch leaves and parents for the given leaf IDs
        let all_leaves: HashMap<TreeNodeId, TreeNode> = self
            .tree_service
            .fetch_leaves_parents(leaf_ids.clone())
            .await?
            .into_iter()
            .map(|node| (node.id.clone(), node))
            .collect();
        for leaf_id in leaf_ids {
            let mut tx_fee_bump_psbts = Vec::new();
            let mut nodes = Vec::new();

            let Some(mut leaf) = all_leaves.get(&leaf_id) else {
                return Err(SparkWalletError::ValidationError(format!(
                    "Leaf ID {leaf_id} not found in the tree",
                )));
            };
            let Some(refund_tx) = &leaf.refund_tx else {
                return Err(SparkWalletError::ValidationError(format!(
                    "Leaf ID {leaf_id} does not have a refund transaction",
                )));
            };

            // Loop through the leaf's ancestors and collect them
            loop {
                nodes.insert(0, leaf);

                let Some(parent_node_id) = &leaf.parent_node_id else {
                    break;
                };
                let Some(parent) = all_leaves.get(parent_node_id) else {
                    return Err(SparkWalletError::ValidationError(format!(
                        "Parent ID {parent_node_id} not found in the tree",
                    )));
                };
                trace!(
                    "Unilateral exit parent {}, txid {}",
                    parent.id,
                    parent.node_tx.compute_txid()
                );
                leaf = parent;
            }

            // For each node check it hasn't already been processed or broadcasted
            for node in nodes {
                let txid = node.node_tx.compute_txid();
                if checked_txs.contains(&txid) {
                    continue;
                }

                checked_txs.insert(txid);
                let is_broadcast = get_transaction_fn(txid.to_string()).await.is_ok();
                if is_broadcast {
                    continue;
                }

                // Create the PSBT to fee bump the node tx
                let psbt = create_fee_bump_psbt(
                    &node.node_tx,
                    &mut utxos,
                    fee_rate,
                    self.config.network.into(),
                )?;

                tx_fee_bump_psbts.push(TxFeeBumpPsbt {
                    tx: node.node_tx.clone(),
                    psbt,
                });

                if node.id == leaf_id {
                    // Create the PSBT to fee bump the leaf refund tx
                    let psbt = create_fee_bump_psbt(
                        refund_tx,
                        &mut utxos,
                        fee_rate,
                        self.config.network.into(),
                    )?;

                    tx_fee_bump_psbts.push(TxFeeBumpPsbt {
                        tx: refund_tx.clone(),
                        psbt,
                    });
                }
            }

            all_leaf_tx_fee_bump_psbts.push(LeafTxFeeBumpPsbts {
                leaf_id,
                tx_fee_bump_psbts,
            });
        }

        Ok(all_leaf_tx_fee_bump_psbts)
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

/// Creates a Partially Signed Bitcoin Transaction (PSBT) to bump the fee of a parent transaction.
///
/// This function creates a PSBT that spends from both input UTXOs and the ephemeral anchor output
/// of the parent transaction. The resulting PSBT can be signed and broadcast to CPFP the parent
/// transaction with a fee.
///
/// # Arguments
/// * `tx` - The parent transaction to be fee bumped
/// * `utxos` - A mutable vector of UTXOs that can be used to pay fees, will be updated with the change UTXO
/// * `fee_rate` - The desired fee rate in satoshis per vbyte
/// * `network` - The Bitcoin network (mainnet, testnet, etc.)
///
/// # Returns
/// A Result containing the PSBT or an error
fn create_fee_bump_psbt(
    tx: &Transaction,
    utxos: &mut Vec<FeeBumpUtxo>,
    fee_rate: u64,
    network: bitcoin::Network,
) -> Result<psbt::Psbt, SparkWalletError> {
    use bitcoin::psbt::{Input as PsbtInput, Output as PsbtOutput, Psbt};

    // Find the ephemeral anchor output in the parent transaction
    let (vout, anchor_tx_out) = tx
        .output
        .iter()
        .enumerate()
        .find(|(_, tx_out)| is_ephemeral_anchor_output(tx_out))
        .ok_or(SparkWalletError::ValidationError(
            "Ephemeral anchor output not found".to_string(),
        ))?;

    // We need at least one UTXO for fee payment
    if utxos.is_empty() {
        return Err(SparkWalletError::ValidationError(
            "At least one UTXO is required for fee bumping".to_string(),
        ));
    }

    // Calculate total available value from all UTXOs
    let total_utxo_value: u64 = utxos.iter().map(|utxo| utxo.value).sum();

    // Use the first UTXO's pubkey for the output
    let first_pubkey = utxos[0].pubkey;
    let output_script_pubkey = Address::p2wpkh(&CompressedPublicKey(first_pubkey), network).into();

    // Create inputs for all UTXOs plus the ephemeral anchor
    let mut inputs = Vec::with_capacity(utxos.len() + 1);

    // Add all UTXO inputs
    for utxo in utxos.iter() {
        inputs.push(TxIn {
            previous_output: OutPoint {
                txid: utxo.txid,
                vout: utxo.vout,
            },
            ..Default::default()
        });
    }

    // Add the ephemeral anchor input
    inputs.push(TxIn {
        previous_output: OutPoint {
            txid: tx.compute_txid(),
            vout: vout as u32,
        },
        ..Default::default()
    });

    // Calculate the approximate transaction size in vbytes
    // P2WPKH inputs: ~68 vbytes each (outpoint + script + witnesses)
    // Anchor input: ~41 vbytes (smaller because no signature needed for ephemeral anchor)
    // P2WPKH output: ~31 vbytes
    // Transaction overhead: ~10 vbytes
    let tx_size_vbytes = (utxos.len() as u64 * 68) + 41 + 31 + 10;
    trace!("Estimated transaction size: {} vbytes", tx_size_vbytes);

    // Calculate fee based on fee rate (fee_rate is in sat/vbyte)
    let fee_amount = fee_rate * tx_size_vbytes;
    trace!("Calculated fee: {} sats", fee_amount);

    // Adjust output value to account for fees
    let adjusted_output_value = total_utxo_value.saturating_sub(fee_amount);
    trace!("Remaining UTXO value: {} sats", adjusted_output_value);

    // Make sure there's enough value to pay the fee
    if adjusted_output_value == 0 {
        return Err(SparkWalletError::ValidationError(
            "UTXOs value is too low to cover the fee".to_string(),
        ));
    }

    // Create the base transaction structure
    let fee_bump_tx = Transaction {
        version: Version::non_standard(3),
        lock_time: LockTime::ZERO,
        input: inputs,
        output: vec![TxOut {
            value: Amount::from_sat(adjusted_output_value),
            script_pubkey: output_script_pubkey,
        }],
    };

    // Create a PSBT from the transaction
    let mut psbt = Psbt::from_unsigned_tx(fee_bump_tx.clone())
        .map_err(|e| SparkWalletError::ValidationError(format!("Failed to create PSBT: {e}")))?;

    // Add PSBT input information for all inputs
    for (i, utxo) in utxos.iter().enumerate() {
        // Add witness UTXO information required for signing
        // This provides information about the output being spent
        let input = PsbtInput {
            witness_utxo: Some(TxOut {
                value: Amount::from_sat(utxo.value),
                script_pubkey: Address::p2wpkh(&CompressedPublicKey(utxo.pubkey), network)
                    .script_pubkey(),
            }),
            ..Default::default()
        };

        psbt.inputs[i] = input;
    }

    // Add information for the last input (the anchor input)
    // Although no signing is needed for the anchor since it uses OP_TRUE,
    // we still provide the witness UTXO information for completeness
    let anchor_input = PsbtInput {
        witness_utxo: Some(anchor_tx_out.clone()),
        ..Default::default()
    };
    psbt.inputs[utxos.len()] = anchor_input;

    // Add details for the output
    psbt.outputs[0] = PsbtOutput::default();

    // Replace all consumed UTXOs with just the change output
    *utxos = vec![FeeBumpUtxo {
        txid: fee_bump_tx.compute_txid(),
        vout: 0,
        value: adjusted_output_value,
        pubkey: first_pubkey,
    }];

    Ok(psbt)
}

pub fn is_ephemeral_anchor_output(tx_out: &TxOut) -> bool {
    tx_out.value.to_sat() == 0 && tx_out.script_pubkey.as_bytes() == [0x51, 0x02, 0x4e, 0x73]
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

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{
        ScriptBuf,
        hashes::Hash,
        secp256k1::{SecretKey, rand},
    };

    /// Creates a transaction with an ephemeral anchor output for testing.
    fn create_test_transaction_with_anchor() -> Transaction {
        // Create a simple transaction with an ephemeral anchor output
        Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: Vec::new(),
            output: vec![TxOut {
                value: Amount::from_sat(0),
                script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]),
            }],
        }
    }

    /// Creates a test UTXO with a random txid and the given pubkey.
    fn create_test_utxo(pubkey: PublicKey, value: u64) -> FeeBumpUtxo {
        // Create a random txid
        let random_bytes = (0..32).map(|_| rand::random::<u8>()).collect::<Vec<_>>();
        let txid = bitcoin::Txid::from_slice(&random_bytes).unwrap();

        FeeBumpUtxo {
            txid,
            vout: 0,
            value,
            pubkey,
        }
    }

    #[test]
    fn test_create_fee_bump_psbt_success() {
        // Create a key pair for testing
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        // Create a transaction with an ephemeral anchor output
        let tx = create_test_transaction_with_anchor();

        // Create a test UTXO with sufficient value
        let mut utxos = vec![create_test_utxo(pubkey, 10_000)];

        // Set a reasonable fee rate (10 sats/vbyte)
        let fee_rate = 10;

        // Call the function
        let result = create_fee_bump_psbt(&tx, &mut utxos, fee_rate, bitcoin::Network::Testnet);

        // Verify the result
        assert!(result.is_ok());

        let psbt = result.unwrap();

        // Validate the PSBT
        assert_eq!(psbt.inputs.len(), 2); // One for our UTXO, one for the anchor
        assert_eq!(psbt.outputs.len(), 1); // Change output

        // Verify the output value accounts for fees
        let estimated_size = 68 + 41 + 31 + 10; // UTXO input + anchor input + output + overhead
        let expected_fee = fee_rate * estimated_size;
        let expected_output_value = 10_000 - expected_fee;

        assert_eq!(
            psbt.unsigned_tx.output[0].value.to_sat(),
            expected_output_value
        );

        // Verify our UTXOs array has been updated with the change output
        assert_eq!(utxos.len(), 1);
        assert_eq!(utxos[0].value, expected_output_value);
        assert_eq!(utxos[0].vout, 0);
    }

    #[test]
    fn test_create_fee_bump_psbt_multiple_utxos() {
        // Create a key pair for testing
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        // Create a transaction with an ephemeral anchor output
        let tx = create_test_transaction_with_anchor();

        // Create multiple test UTXOs
        let mut utxos = vec![
            create_test_utxo(pubkey, 5_000),
            create_test_utxo(pubkey, 3_000),
            create_test_utxo(pubkey, 2_000),
        ];

        // Set a reasonable fee rate
        let fee_rate = 10;

        // Call the function
        let result = create_fee_bump_psbt(&tx, &mut utxos, fee_rate, bitcoin::Network::Testnet);

        // Verify the result
        assert!(result.is_ok());

        let psbt = result.unwrap();

        // Validate the PSBT
        assert_eq!(psbt.inputs.len(), 4); // Three UTXOs + anchor
        assert_eq!(psbt.outputs.len(), 1); // Change output

        // Verify the total input value (excluding anchor which is 0)
        let total_input_value = 5_000 + 3_000 + 2_000;

        // Verify the output value accounts for fees
        let estimated_size = (3 * 68) + 41 + 31 + 10; // 3 UTXO inputs + anchor input + output + overhead
        let expected_fee = fee_rate * estimated_size;
        let expected_output_value = total_input_value - expected_fee;

        assert_eq!(
            psbt.unsigned_tx.output[0].value.to_sat(),
            expected_output_value
        );

        // Verify our UTXOs array has been updated with the change output
        assert_eq!(utxos.len(), 1);
        assert_eq!(utxos[0].value, expected_output_value);
    }

    #[test]
    fn test_create_fee_bump_psbt_no_utxos() {
        // Create a transaction with an ephemeral anchor output
        let tx = create_test_transaction_with_anchor();

        // Empty UTXOs vector
        let mut utxos = Vec::new();

        // Call the function
        let result = create_fee_bump_psbt(&tx, &mut utxos, 10, bitcoin::Network::Testnet);

        // Verify the PSBT creation fails
        assert!(result.is_err());
    }

    #[test]
    fn test_create_fee_bump_psbt_insufficient_value() {
        // Create a key pair for testing
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        // Create a transaction with an ephemeral anchor output
        let tx = create_test_transaction_with_anchor();

        // Create a test UTXO with very low value
        let mut utxos = vec![create_test_utxo(pubkey, 10)];

        // Set a high fee rate to ensure the fee exceeds the UTXO value
        let fee_rate = 100;

        // Call the function
        let result = create_fee_bump_psbt(&tx, &mut utxos, fee_rate, bitcoin::Network::Testnet);

        // Verify the PSBT creation fails
        assert!(result.is_err());
    }

    #[test]
    fn test_create_fee_bump_psbt_no_anchor_output() {
        // Create a key pair for testing
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let pubkey = PublicKey::from_secret_key(&secp, &secret_key);

        // Create a transaction WITHOUT an anchor output (just a regular output)
        let tx = Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: Vec::new(),
            output: vec![TxOut {
                value: Amount::from_sat(1000),
                script_pubkey: Address::p2wpkh(
                    &CompressedPublicKey(pubkey),
                    bitcoin::Network::Testnet,
                )
                .script_pubkey(),
            }],
        };

        let mut utxos = vec![create_test_utxo(pubkey, 10_000)];

        // Call the function
        let result = create_fee_bump_psbt(&tx, &mut utxos, 10, bitcoin::Network::Testnet);

        // Should fail because no anchor output was found
        assert!(result.is_err());
        if let Err(SparkWalletError::ValidationError(msg)) = result {
            assert!(msg.contains("Ephemeral anchor output not found"));
        } else {
            panic!("Expected ValidationError");
        }
    }

    #[test]
    fn test_is_ephemeral_anchor_output() {
        // Test case 1: Valid ephemeral anchor output
        let valid_anchor = TxOut {
            value: Amount::from_sat(0),
            script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]),
        };
        assert!(is_ephemeral_anchor_output(&valid_anchor));

        // Test case 2: Non-zero value
        let non_zero_value = TxOut {
            value: Amount::from_sat(1),
            script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]),
        };
        assert!(!is_ephemeral_anchor_output(&non_zero_value));

        // Test case 3: Different script
        let different_script = TxOut {
            value: Amount::from_sat(0),
            script_pubkey: ScriptBuf::from(vec![0x51]),
        };
        assert!(!is_ephemeral_anchor_output(&different_script));
    }
}
