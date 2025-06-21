use bitcoin::{
    Address, Transaction,
    hashes::{Hash, sha256},
    params::Params,
    secp256k1::PublicKey,
};
use std::{collections::HashMap, time::Duration};
use uuid::Uuid;

use spark::{
    bitcoin::BitcoinService,
    leaves::LeafManager,
    operator::rpc::{ConnectionManager, SparkRpcClient},
    services::{DepositAddress, DepositService, LeafKeyTweak, Transfer, TransferService},
    signer::Signer,
    tree::{TreeNode, TreeNodeStatus},
};

use crate::leaf::WalletLeaf;

use super::{SparkWalletConfig, SparkWalletError};

pub struct SparkWallet<S>
where
    S: Signer + Clone,
{
    config: SparkWalletConfig,
    deposit_service: DepositService<S>,
    leaf_manager: LeafManager,
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

        let deposit_service = DepositService::new(
            bitcoin_service,
            spark_rpc_client,
            identity_public_key,
            config.network,
            config.operator_pool.clone(),
            signer.clone(),
        );

        let transfer_service = TransferService::new(signer.clone());
        let leaf_manager = LeafManager::new();
        Ok(SparkWallet {
            deposit_service,
            config,
            leaf_manager,
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
    ) -> Result<Vec<WalletLeaf>, SparkWalletError> {
        // TODO: This entire function happens inside a txid mutex in the js sdk. It seems unnecessary here?

        // TODO: It seems like the unused deposit addresses could be cached in the wallet, so we don't have to query them every time.
        let unused_addresses = self
            .deposit_service
            .query_unused_deposit_addresses()
            .await?;
        let unused_addresses: HashMap<Address, DepositAddress> = unused_addresses
            .into_iter()
            .map(|addr| (addr.address.clone(), addr))
            .collect();
        let params: Params = self.config.network.into();

        // TODO: Ensure all inputs are segwit inputs, so this tx is not malleable.
        // Normally the tx should be already confirmed, but perhaps we get in trouble with a reorg?
        for (vout, output) in tx.output.iter().enumerate() {
            let Ok(address) = Address::from_script(&output.script_pubkey, &params) else {
                continue;
            };

            let Some(deposit_address) = unused_addresses.get(&address) else {
                continue;
            };

            let signing_pubkey = self
                .signer
                .generate_public_key(sha256::Hash::hash(deposit_address.leaf_id.as_bytes()))?;
            // TODO: If leaf id is actually optional:
            //   let signingPubKey: Uint8Array;
            //   if (!depositAddress.leafId) {
            //     signingPubKey = depositAddress.userSigningPublicKey;
            //   } else {
            //     signingPubKey = await this.config.signer.generatePublicKey(
            //       sha256(depositAddress.leafId),
            //     );
            //   }

            let nodes = self
                .finalize_deposit(&signing_pubkey, deposit_address, tx, vout as u32)
                .await?;

            return Ok(nodes.into_iter().map(WalletLeaf::from).collect());
        }

        Err(SparkWalletError::DepositAddressUsed)
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

        // TODO: Seems below can be more efficient.
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
            Some(pending_transfer) => {
                self.claim_transfer(&pending_transfer, false, 0, false)
                    .await?
            }
            None => vec![],
        };

        self.leaf_manager.add_leaves(&result_nodes).await;
        self.leaf_manager.remove_leaves(&leaves).await;

        Ok(result_nodes)
    }

    async fn claim_transfer(
        &self,
        transfer: &Transfer,
        emit: bool,
        retry_count: u32,
        optimize: bool,
    ) -> Result<Vec<TreeNode>, SparkWalletError> {
        let max_retries = 5;
        let base_delay_ms = 1000;
        let max_delay_ms = 10000;

        // TODO: Does this have to me run inside a mutex? The js sdk does this.

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
        let Ok(leaf_pubkey_map) = self
            .transfer_service
            .verify_pending_transfer(transfer)
            .await
        else {
            return Box::pin(self.claim_transfer(transfer, emit, retry_count + 1, optimize)).await;
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
        let Ok(result) = self
            .transfer_service
            .claim_transfer(transfer, leaves_to_claim)
            .await
        else {
            return Box::pin(self.claim_transfer(transfer, emit, retry_count + 1, optimize)).await;
        };

        // TODO: If emit is true, emit an event here.

        // TODO: Is this the right place to check timelocks? Perhaps a leaf manager should handle this?
        let result = self.check_refresh_timelock_nodes(result).await?;
        let result = self.check_extend_timelock_nodes(result).await?;

        self.leaf_manager.add_leaves(&result).await;

        // TODO: Optimize leaves if optimize is true and the transfer type is not counter swap. (or make leaf manager handle this)

        Ok(result)
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
