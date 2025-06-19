use bitcoin::{
    Address, Transaction,
    hashes::{Hash, sha256},
    params::Params,
    secp256k1::PublicKey,
};
use std::collections::HashMap;
use uuid::Uuid;

use spark::{
    bitcoin::BitcoinService,
    operator::rpc::{ConnectionManager, SparkRpcClient},
    services::{DepositAddress, DepositService},
    signer::Signer,
};

use crate::leaf::WalletLeaf;

use super::{SparkWalletConfig, SparkWalletError};

pub struct SparkWallet<S>
where
    S: Signer + Clone,
{
    config: SparkWalletConfig,
    deposit_service: DepositService<S>,
    signer: S,
}

impl<S: Signer + Clone> SparkWallet<S> {
    pub async fn new(config: SparkWalletConfig, signer: S) -> Result<Self, SparkWalletError> {
        let identity_public_key = signer.get_identity_public_key(0, config.network)?;
        let connection_manager = ConnectionManager::new();
        let spark_service_channel = connection_manager
            .get_channel(&config.operator_pool.get_coordinator())
            .await?;
        let bitcoin_service = BitcoinService::new(config.network);
        let spark_rpc_client = SparkRpcClient::new(spark_service_channel, signer.clone());
        let deposit_service = DepositService::new(
            bitcoin_service,
            spark_rpc_client,
            identity_public_key,
            config.network,
            config.operator_pool.clone(),
            signer.clone(),
        );

        Ok(SparkWallet {
            deposit_service,
            config,
            signer,
        })
    }

    // TODO: In the js sdk this function calls an electrum server to fetch the transaction hex based on a txid.
    // Intuitively this function is being called when you've already learned about a transaction, so it could be passed in directly.
    pub async fn claim_deposit(
        &self,
        tx: Transaction,
    ) -> Result<Vec<WalletLeaf>, SparkWalletError> {
        // TODO: This entire function happens inside a txid mutex in the js sdk. It seems unnecessary here?
        let unused_addresses = self
            .deposit_service
            .query_unused_deposit_addresses()
            .await?;
        let unused_addresses: HashMap<Address, DepositAddress> = unused_addresses
            .into_iter()
            .map(|addr| (addr.address.clone(), addr))
            .collect();
        let params: Params = self.config.network.into();
        for (vout, output) in tx.output.iter().enumerate() {
            let Ok(address) = Address::from_script(&output.script_pubkey, &params) else {
                continue;
            };

            let Some(deposit_address) = unused_addresses.get(&address) else {
                continue;
            };

            let signing_pubkey = self
                .signer
                .generate_public_key(sha256::Hash::hash(deposit_address.leaf_id.as_bytes()))
                .await?;
            // TODO: If leaf id is actually optional:
            //   let signingPubKey: Uint8Array;
            //   if (!depositAddress.leafId) {
            //     signingPubKey = depositAddress.userSigningPublicKey;
            //   } else {
            //     signingPubKey = await this.config.signer.generatePublicKey(
            //       sha256(depositAddress.leafId),
            //     );
            //   }

            return self
                .finalize_deposit(&signing_pubkey, deposit_address, tx, vout as u32)
                .await;
        }

        Err(SparkWalletError::DepositAddressUsed)
    }

    async fn finalize_deposit(
        &self,
        signing_public_key: &PublicKey,
        address: &DepositAddress,
        tx: Transaction,
        vout: u32,
    ) -> Result<Vec<WalletLeaf>, SparkWalletError> {
        let res = self
            .deposit_service
            .create_tree_root(signing_public_key, &address.verifying_public_key, tx, vout)
            .await?;
        todo!()
        // const resultingNodes: TreeNode[] = [];
        // for (const node of res.nodes) {
        //   if (node.status === "AVAILABLE") {
        //     const { nodes } = await this.transferService.extendTimelock(
        //       node,
        //       signingPubKey,
        //     );

        //     for (const n of nodes) {
        //       if (n.status === "AVAILABLE") {
        //         const transfer = await this.transferLeavesToSelf(
        //           [n],
        //           signingPubKey,
        //         );
        //         resultingNodes.push(...transfer);
        //       } else {
        //         resultingNodes.push(n);
        //       }
        //     }
        //   } else {
        //     resultingNodes.push(node);
        //   }
        // }

        // return resultingNodes;
    }

    pub async fn generate_deposit_address(
        &self,
        is_static: bool,
    ) -> Result<Address, SparkWalletError> {
        let leaf_id = Uuid::now_v7();
        let hash = sha256::Hash::hash(leaf_id.as_bytes());
        let signing_public_key = self.signer.generate_public_key(hash).await?;
        let address = self
            .deposit_service
            .generate_deposit_address(signing_public_key, leaf_id.to_string(), is_static)
            .await?;
        Ok(address.address)
    }
}
