use std::sync::Arc;

use bitcoin::{Address, Transaction};

use spark::{
    address::SparkAddress,
    bitcoin::BitcoinService,
    operator::rpc::{ConnectionManager, SparkRpcClient},
    services::{DepositService, Transfer, TransferService},
    signer::Signer,
    ssp::ServiceProvider,
    tree::{TreeNode, TreeNodeId, TreeService, TreeState},
};

use crate::{leaf::WalletLeaf, model::WalletTransfer};

use super::{SparkWalletConfig, SparkWalletError};

pub struct SparkWallet<S>
where
    S: Signer + Clone,
{
    config: SparkWalletConfig,
    deposit_service: DepositService<S>,
    signer: S,
    tree_service: TreeService<S>,
    transfer_service: Arc<TransferService<S>>,
}

impl<S: Signer + Clone> SparkWallet<S> {
    pub async fn new(config: SparkWalletConfig, signer: S) -> Result<Self, SparkWalletError> {
        let identity_public_key = signer.get_identity_public_key()?;
        let connection_manager = ConnectionManager::new();

        // spark operator
        let spark_service_channel = connection_manager
            .get_channel(config.operator_pool.get_coordinator())
            .await?;
        let bitcoin_service = BitcoinService::new(config.network);
        let spark_rpc_client =
            SparkRpcClient::new(spark_service_channel, config.network, signer.clone());
        let _service_provider = ServiceProvider::new(
            config.service_provider_config.clone(),
            config.network,
            signer.clone(),
        );

        // spark ssp
        let ssp_client = Arc::new(ServiceProvider::new(
            // TODO: Should be taken from config
            ServiceProviderOptions {
                base_url: "".to_string(),
                schema_endpoint: None,
                identity_public_key,
            },
            config.network,
            signer.clone(),
        ));

        let bitcoin_service = BitcoinService::new(config.network);
        let deposit_service = DepositService::new(
            bitcoin_service,
            spark_rpc_client.clone(),
            identity_public_key,
            config.network,
            config.operator_pool.clone(),
            signer.clone(),
        );

        let transfer_service = Arc::new(TransferService::new(signer.clone()));
        let tree_state = TreeState::new();
        let tree_service = TreeService::new(tree_state, Arc::clone(&transfer_service));

        Ok(SparkWallet {
            config,
            deposit_service,
            signer,
            tree_service,
            transfer_service,
        })
    }

    pub async fn pay_lightning_invoice(
        &self,
        invoice: &String,
    ) -> Result<LightningSendPayment, SparkWalletError> {
        let leaves = self.leaf_manager.get_leaves().await;
        PayLightningInvoice::new(
            self.lightning_service.clone(),
            self.transfer_service.clone(),
            invoice.clone(),
            leaves.clone(),
        )
        .execute()
        .await
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
        let optimized_nodes = self.tree_service.collect_leaves(deposit_nodes).await?;
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
        let leaves = self
            .tree_service
            .select_leaves_by_amount(amount_sat)
            .await?;

        // TODO: do we need to refresh and or extend timelocks? js sdk does this

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
        let transfers = self.transfer_service.query_pending_transfers().await?;

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
        let result_nodes = self.transfer_service.claim_transfer(transfer, None).await?;

        // update local tree state (may be optimized to only add leaves that were received)
        self.tree_service.refresh_leaves().await?;

        // TODO: Emit events if emit is true
        // TODO: Optimize leaves if optimize is true and the transfer type is not counter swap

        Ok(result_nodes)
    }
}
