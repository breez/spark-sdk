use bitcoin::{Address, Transaction};

use spark::{
    bitcoin::BitcoinService,
    operator::rpc::{ConnectionManager, SparkRpcClient},
    services::{DepositService, TransferService},
    signer::Signer,
    ssp::ServiceProvider,
    tree::{TreeNode, TreeNodeId, TreeService, TreeState},
};

use crate::leaf::WalletLeaf;

use super::{SparkWalletConfig, SparkWalletError};

pub struct SparkWallet<S>
where
    S: Signer + Clone,
{
    deposit_service: DepositService<S>,
    signer: S,
    tree_service: TreeService<S>,
}

impl<S: Signer + Clone> SparkWallet<S> {
    pub async fn new(config: SparkWalletConfig, signer: S) -> Result<Self, SparkWalletError> {
        let identity_public_key = signer.get_identity_public_key(0)?;
        let connection_manager = ConnectionManager::new();
        let spark_service_channel = connection_manager
            .get_channel(config.operator_pool.get_coordinator())
            .await?;
        let bitcoin_service = BitcoinService::new(config.network);
        let spark_rpc_client =
            SparkRpcClient::new(spark_service_channel, config.network, signer.clone());
        let service_provider = ServiceProvider::new(
            config.service_provider_config.clone(),
            config.network,
            signer.clone(),
        );

        let deposit_service = DepositService::new(
            bitcoin_service,
            spark_rpc_client,
            identity_public_key,
            config.network,
            config.operator_pool.clone(),
            signer.clone(),
        );

        let transfer_service = TransferService::new(signer.clone());
        let tree_state = TreeState::new();
        let tree_service = TreeService::new(tree_state, transfer_service);
        Ok(SparkWallet {
            deposit_service,
            signer,
            tree_service,
        })
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

    async fn check_extend_timelock_nodes(
        &self,
        nodes: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, SparkWalletError> {
        todo!()
    }

    async fn check_refresh_timelock_nodes(
        &self,
        nodes: Vec<TreeNode>,
    ) -> Result<Vec<TreeNode>, SparkWalletError> {
        todo!()
    }
}
