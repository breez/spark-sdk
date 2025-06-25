use bitcoin::{
    Address, Transaction, TxOut,
    hashes::{Hash, sha256},
    params::Params,
    secp256k1::PublicKey,
};
use std::time::Duration;
use uuid::Uuid;

use spark::{
    bitcoin::BitcoinService,
    operator::rpc::{ConnectionManager, SparkRpcClient},
    services::{DepositAddress, DepositService, LeafKeyTweak, Transfer, TransferService},
    signer::Signer,
    ssp::ServiceProvider,
    tree::{TreeNode, TreeNodeStatus, TreeState},
};

use crate::leaf::WalletLeaf;

use super::{SparkWalletConfig, SparkWalletError};

pub struct SparkWallet<S>
where
    S: Signer + Clone,
{
    config: SparkWalletConfig,
    service_provider: ServiceProvider<S>,
    deposit_service: DepositService<S>,
    tree_state: TreeState,
    signer: S,
    transfer_service: TransferService<S>,
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
        Ok(SparkWallet {
            service_provider,
            deposit_service,
            config,
            tree_state,
            signer,
            transfer_service,
        })
    }

    // TODO: In the js sdk this function calls an electrum server to fetch the transaction hex based on a txid.
    // Intuitively this function is being called when you've already learned about a transaction, so it could be passed in directly.
    /// Claims a deposit by finding the first unused deposit address in the transaction outputs.
    pub async fn claim_deposit(
        &self,
        tx: Transaction,
        vout: usize,
    ) -> Result<Vec<WalletLeaf>, SparkWalletError> {
        // TODO: This entire function happens inside a txid mutex in the js sdk. It seems unnecessary here?
        // TODO: Ensure all inputs are segwit inputs, so this tx is not malleable. Normally the tx should be already confirmed, but perhaps we get in trouble with a reorg?

        let params: Params = self.config.network.into();

        let output: &TxOut = tx
            .output
            .get(vout)
            .ok_or(SparkWalletError::InvalidOutputIndex)?;
        let address = Address::from_script(&output.script_pubkey, params)
            .map_err(|_| SparkWalletError::NotADepositOutput)?;
        let deposit_address = self
            .deposit_service
            .get_unused_deposit_address(&address)
            .await?
            .ok_or(SparkWalletError::DepositAddressUsed)?;
        let signing_pubkey = self
            .signer
            .generate_public_key(sha256::Hash::hash(deposit_address.leaf_id.as_bytes()))?;
        let nodes = self
            .finalize_deposit(&signing_pubkey, &deposit_address, tx, vout as u32)
            .await?;

        Ok(nodes.into_iter().map(WalletLeaf::from).collect())
    }

    async fn finalize_deposit(
        &self,
        signing_public_key: &PublicKey,
        address: &DepositAddress,
        tx: Transaction,
        vout: u32,
    ) -> Result<Vec<TreeNode>, SparkWalletError> {
        let nodes = self
            .deposit_service
            .create_tree_root(signing_public_key, &address.verifying_public_key, tx, vout)
            .await?;

        // TODO: The `create_tree_root` result should probably be persisted in case below calls fail. Persisting should include the transactions.

        // TODO: This step is leaf optimization. This should go in a separate service.
        let mut resulting_nodes = Vec::new();
        for node in nodes {
            if node.status != TreeNodeStatus::Available {
                resulting_nodes.push(node);
                continue;
            }

            let nodes = self
                .transfer_service
                .extend_time_lock(&node, signing_public_key)
                .await?;

            for n in nodes {
                if n.status == TreeNodeStatus::Available {
                    let transfer = self
                        .transfer_leaves_to_self(vec![n], signing_public_key)
                        .await?;
                    resulting_nodes.extend(transfer.into_iter());
                } else {
                    resulting_nodes.push(n);
                }
            }
        }

        Ok(resulting_nodes)
    }

    pub async fn generate_deposit_address(
        &self,
        is_static: bool,
    ) -> Result<Address, SparkWalletError> {
        let leaf_id = Uuid::now_v7();
        let hash = sha256::Hash::hash(leaf_id.as_bytes());
        let signing_public_key = self.signer.generate_public_key(hash)?;
        let address = self
            .deposit_service
            .generate_deposit_address(signing_public_key, leaf_id.to_string(), is_static)
            .await?;

        // TODO: Watch this address for deposits.

        Ok(address.address)
    }

    async fn transfer_leaves_to_self(
        &self,
        leaves: Vec<TreeNode>,
        signing_public_key: &PublicKey,
    ) -> Result<Vec<TreeNode>, SparkWalletError> {
        let leaf_key_tweaks = leaves
            .iter()
            .map(|leaf| {
                let new_signing_public_key = self
                    .signer
                    .generate_public_key(sha256::Hash::hash(leaf.id.as_bytes()))?;
                Ok(LeafKeyTweak {
                    node: leaf.clone(),
                    signing_public_key: *signing_public_key,
                    new_signing_public_key,
                })
            })
            .collect::<Result<Vec<_>, SparkWalletError>>()?;

        let transfer = self
            .transfer_service
            .send_transfer_with_key_tweaks(leaf_key_tweaks, signing_public_key)
            .await?;

        // TODO: Why is the transfer queried again after the send_transfer_with_key_tweaks above?
        let pending_transfer = self.transfer_service.query_transfer(&transfer.id).await?;

        // TODO: Validate the pending transfer contains the leaves we expect to transfer.

        let result_nodes = match pending_transfer {
            Some(pending_transfer) => self.claim_transfer(&pending_transfer, false, false).await?,
            None => vec![],
        };

        self.tree_state.add_leaves(&result_nodes).await;
        self.tree_state.remove_leaves(&leaves).await;

        Ok(result_nodes)
    }

    async fn claim_transfer(
        &self,
        transfer: &Transfer,
        emit: bool,
        optimize: bool,
    ) -> Result<Vec<TreeNode>, SparkWalletError> {
        let max_retries = 5;
        let base_delay_ms = 1000;
        let max_delay_ms = 10000;

        // TODO: Does this have to me run inside a mutex? The js sdk does this.

        let mut retry_count = 0;
        loop {
            if retry_count >= max_retries {
                // TODO: Return the last error instead of a generic error.
                return Err(SparkWalletError::Generic(
                    "max retries exceeded".to_string(),
                ));
            }

            // Introduce an exponential backoff delay before retrying.
            if retry_count > 0 {
                let delay_ms = (base_delay_ms * 2u64.pow(retry_count - 1)).min(max_delay_ms);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }

            // TODO: Is this step really necessary? We expect to be claiming these leaves. If they don't exist on the remote, there is a problem. It seems like we shouldn't just ignore the missing ones.
            let leaf_pubkey_map = match self
                .transfer_service
                .verify_pending_transfer(transfer)
                .await
            {
                Ok(map) => map,
                Err(_) => {
                    retry_count += 1;
                    continue;
                }
            };

            let mut leaves_to_claim = Vec::new();
            for leaf in &transfer.leaves {
                let Some(leaf_pubkey) = leaf_pubkey_map.get(&leaf.leaf.id) else {
                    continue;
                };
                leaves_to_claim.push(LeafKeyTweak {
                    node: leaf.leaf.clone(),
                    signing_public_key: *leaf_pubkey,
                    new_signing_public_key: self
                        .signer
                        .generate_public_key(sha256::Hash::hash(leaf.leaf.id.as_bytes()))?,
                });
            }

            if leaves_to_claim.is_empty() {
                return Ok(Vec::new());
            }

            // TODO: Validate the resulting leaves are the ones we expect to claim.
            let result = match self
                .transfer_service
                .claim_transfer(transfer, leaves_to_claim)
                .await
            {
                Ok(res) => res,
                Err(_) => {
                    retry_count += 1;
                    continue;
                }
            };

            // TODO: If emit is true, emit an event here.
            // TODO: Is this the right place to check timelocks? Perhaps a leaf manager should handle this?

            let result = self.check_refresh_timelock_nodes(result).await?;
            let result = self.check_extend_timelock_nodes(result).await?;
            self.tree_state.add_leaves(&result).await;

            // TODO: Optimize leaves if optimize is true and the transfer type is not counter swap. (or make leaf manager handle this)

            return Ok(result);
        }
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
