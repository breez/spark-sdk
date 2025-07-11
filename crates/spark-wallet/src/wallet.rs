use std::{collections::HashMap, sync::Arc};

use bitcoin::{Address, Transaction};

use spark::{
    address::SparkAddress,
    bitcoin::BitcoinService,
    operator::rpc::{ConnectionManager, SparkRpcClient},
    services::{
        DepositService, LightningReceivePayment, LightningSendPayment, LightningService,
        PagingFilter, Swap, Transfer, TransferService,
    },
    signer::Signer,
    ssp::ServiceProvider,
    tree::{TreeNode, TreeNodeId, TreeService, TreeState},
};
use tracing::{debug, trace};

use crate::{
    leaf::WalletLeaf,
    model::{WalletInfo, WalletTransfer},
};

use super::{SparkWalletConfig, SparkWalletError};

pub struct SparkWallet<S>
where
    S: Signer + Clone,
{
    config: SparkWalletConfig,
    deposit_service: DepositService<S>,
    signer: S,
    swap_service: Arc<Swap<S>>,
    tree_service: TreeService<S>,
    transfer_service: Arc<TransferService<S>>,
    lightning_service: Arc<LightningService<S>>,
}

impl<S: Signer + Clone> SparkWallet<S> {
    pub async fn new(config: SparkWalletConfig, signer: S) -> Result<Self, SparkWalletError> {
        config.validate()?;
        let identity_public_key = signer.get_identity_public_key()?;
        let connection_manager = ConnectionManager::new();

        let (signing_operators_clients, coordinator_client) =
            Self::init_operator_clients(&config, &connection_manager, signer.clone()).await?;

        let bitcoin_service = BitcoinService::new(config.network);
        let service_provider = Arc::new(ServiceProvider::new(
            config.service_provider_config.clone(),
            signer.clone(),
        ));

        let lightning_service = Arc::new(LightningService::new(
            coordinator_client.clone(),
            signing_operators_clients.clone(),
            service_provider.clone(),
            config.network,
            signer.clone(),
            config.split_secret_threshold,
        ));
        let deposit_service = DepositService::new(
            bitcoin_service,
            coordinator_client.clone(),
            identity_public_key,
            config.network,
            config.operator_pool.clone(),
            signer.clone(),
        );

        let transfer_service = Arc::new(TransferService::new(
            signer.clone(),
            config.network,
            config.split_secret_threshold,
            coordinator_client.clone(),
            signing_operators_clients.clone(),
        ));
        let tree_state = TreeState::new();
        let tree_service = TreeService::new(
            coordinator_client.clone(),
            identity_public_key,
            config.network,
            tree_state,
            Arc::clone(&transfer_service),
        );

        let swap_service = Arc::new(Swap::new(
            coordinator_client.clone(),
            config.network,
            signing_operators_clients,
            signer.clone(),
            config.split_secret_threshold,
            Arc::clone(&service_provider),
            Arc::clone(&transfer_service),
        ));
        Ok(SparkWallet {
            config,
            deposit_service,
            signer,
            swap_service,
            tree_service,
            transfer_service,
            lightning_service,
        })
    }

    async fn init_operator_clients(
        config: &SparkWalletConfig,
        connection_manager: &ConnectionManager,
        signer: S,
    ) -> Result<(Vec<Arc<SparkRpcClient<S>>>, Arc<SparkRpcClient<S>>), SparkWalletError> {
        let mut signing_operators_clients = vec![];
        for operator in config.operator_pool.get_all_operators() {
            let channel = connection_manager.get_channel(operator).await?;
            let client = Arc::new(SparkRpcClient::new(
                channel,
                config.network,
                signer.clone(),
                operator.clone(),
            ));
            signing_operators_clients.push(client);
        }
        let coordinator = config.operator_pool.get_coordinator().clone();
        let channel = connection_manager.get_channel(&coordinator).await?;
        let coordinator_client = Arc::new(SparkRpcClient::new(
            channel,
            config.network,
            signer.clone(),
            coordinator,
        ));
        Ok((signing_operators_clients, coordinator_client))
    }

    pub async fn list_leaves(&self) -> Result<Vec<WalletLeaf>, SparkWalletError> {
        let leaves = self.tree_service.list_leaves().await?;
        Ok(leaves.into_iter().map(WalletLeaf::from).collect())
    }

    pub async fn pay_lightning_invoice(
        &self,
        invoice: &str,
        max_fee_sat: Option<u64>,
    ) -> Result<LightningSendPayment, SparkWalletError> {
        let total_amount_sat = self
            .lightning_service
            .validate_payment(invoice, max_fee_sat)
            .await?;

        let leaves = self.select_leaves(total_amount_sat).await?;

        // start the lightning swap with the operator
        let swap = self
            .lightning_service
            .start_lightning_swap(invoice, &leaves)
            .await?;

        // send the leaves to the operator
        let _ = self
            .transfer_service
            .deliver_transfer_package(&swap.transfer, &swap.leaves, HashMap::new())
            .await?;

        // finalize the lightning swap with the ssp - send the actual lightning payment
        Ok(self
            .lightning_service
            .finalize_lightning_swap(&swap)
            .await?)
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

    pub async fn fetch_lightning_send_fee_estimate(
        &self,
        invoice: &str,
    ) -> Result<u64, SparkWalletError> {
        Ok(self
            .lightning_service
            .fetch_lightning_send_fee_estimate(invoice)
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
        debug!("Claimed deposit nodes: {:?}", deposit_nodes);
        let optimized_nodes = self.tree_service.collect_leaves(deposit_nodes).await?;
        debug!("Optimized nodes: {:?}", optimized_nodes);
        Ok(optimized_nodes.into_iter().map(WalletLeaf::from).collect())
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

    pub async fn list_unused_deposit_addresses(&self) -> Result<Vec<Address>, SparkWalletError> {
        let deposit_addresses = self
            .deposit_service
            .query_unused_deposit_addresses(&PagingFilter::default())
            .await?;
        Ok(deposit_addresses
            .items
            .into_iter()
            .map(|addr| addr.address)
            .collect())
    }

    pub async fn swap_leaves(
        &self,
        leaf_ids: Vec<TreeNodeId>,
        target_amounts: Vec<u64>,
    ) -> Result<Vec<WalletLeaf>, SparkWalletError> {
        let leaves: Vec<_> = self
            .tree_service
            .list_leaves()
            .await?
            .into_iter()
            .filter(|leaf| leaf_ids.contains(&leaf.id))
            .collect();
        if leaves.len() != leaf_ids.len() {
            return Err(SparkWalletError::LeavesNotFound);
        }
        let transfer = self
            .swap_service
            .swap_leaves(leaves, target_amounts)
            .await?;
        let leaves = self.claim_transfer(&transfer, false, false).await?;
        Ok(leaves.into_iter().map(WalletLeaf::from).collect())
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

        // TODO: is there a good reason to allow self-transfers?
        let is_self_transfer = receiver_pubkey == self.signer.get_identity_public_key()?;

        // get leaves to transfer
        let leaves = self.select_leaves(amount_sat).await?;

        let transfer = self
            .transfer_service
            .transfer_leaves_to(&leaves, &receiver_pubkey)
            .await?;

        // if self-transfer, claim it immediately
        if is_self_transfer {
            // TODO: do we need to re-fetch the transfer? js sdk does this
            self.claim_transfer(&transfer, false, false).await?;
        }

        // update local tree state (may be optimized to only drop leaves that were transferred + potentially add new leaves if self-transfer)
        self.tree_service.refresh_leaves().await?;

        Ok(transfer.into())
    }

    /// Claims all pending transfers.
    pub async fn claim_pending_transfers(&self) -> Result<Vec<WalletTransfer>, SparkWalletError> {
        trace!("Claiming all pending transfers");
        let transfers = self
            .transfer_service
            .query_pending_receiver_transfers(&PagingFilter::default())
            .await?;
        trace!("There are {} pending transfers", transfers.len());
        for transfer in &transfers {
            self.claim_transfer(transfer, false, false).await?;
        }

        Ok(transfers.into_iter().map(WalletTransfer::from).collect())
    }

    async fn claim_transfer(
        &self,
        transfer: &Transfer,
        _emit: bool,
        _optimize: bool,
    ) -> Result<Vec<TreeNode>, SparkWalletError> {
        trace!("Claiming transfer with id: {}", transfer.id);
        let result_nodes = self.transfer_service.claim_transfer(transfer, None).await?;

        trace!("Refreshing leaves after claiming transfer");
        // update local tree state (may be optimized to only add leaves that were received)
        self.tree_service.refresh_leaves().await?;

        // TODO: Emit events if emit is true
        // TODO: Optimize leaves if optimize is true and the transfer type is not counter swap

        Ok(result_nodes)
    }

    pub async fn get_info(&self) -> Result<WalletInfo, SparkWalletError> {
        Ok(WalletInfo {
            identity_public_key: self.signer.get_identity_public_key()?,
            network: self.config.network,
        })
    }

    pub async fn get_spark_address(&self) -> Result<SparkAddress, SparkWalletError> {
        Ok(SparkAddress {
            identity_public_key: self.signer.get_identity_public_key()?,
            network: self.config.network,
        })
    }

    pub async fn get_balance(&self) -> Result<u64, SparkWalletError> {
        Ok(self.tree_service.get_available_balance().await?)
    }

    pub async fn list_transfers(&self) -> Result<Vec<WalletTransfer>, SparkWalletError> {
        let transfers = self
            .transfer_service
            .query_all_transfers(&PagingFilter::default())
            .await?;
        Ok(transfers.into_iter().map(WalletTransfer::from).collect())
    }

    pub async fn list_pending_transfers(&self) -> Result<Vec<WalletTransfer>, SparkWalletError> {
        let transfers = self
            .transfer_service
            .query_pending_transfers(&PagingFilter::default())
            .await?;
        Ok(transfers.into_iter().map(WalletTransfer::from).collect())
    }

    /// Selects leaves from the tree that sum up to exactly the target amount.
    /// If such a combination of leaves does not exist, it performs a swap to get a set of leaves matching the target amount.
    /// If no leaves can be selected, returns an error
    async fn select_leaves(
        &self,
        target_amount_sat: u64,
    ) -> Result<Vec<TreeNode>, SparkWalletError> {
        let leaves = self.tree_service.select_leaves_by_amount(target_amount_sat).await?;
        if let Some(leaves) = leaves {
            return Ok(leaves)
        }

        let leaves = self.tree_service.select_leaves_by_minimum_amount(target_amount_sat).await?;
        let Some(leaves) = leaves else {
            return Err(SparkWalletError::InsufficientFunds)
        };

        self.swap_leaves(leaves.into_iter().map(|leaf|leaf.id).collect(), vec![target_amount_sat]).await?;
        
        let leaves = self.tree_service.select_leaves_by_amount(target_amount_sat).await?;
        let leaves = leaves.ok_or(SparkWalletError::InsufficientFunds)?;

        Ok(leaves)
    }

    pub async fn sync(&self) -> Result<(), SparkWalletError> {
        self.tree_service.refresh_leaves().await?;
        Ok(())
    }
}
