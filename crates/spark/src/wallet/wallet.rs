use std::collections::HashMap;

use bitcoin::{
    Address, Transaction,
    hashes::{Hash, sha256},
};
use uuid::Uuid;

use crate::{
    Network,
    services::{DepositAddress, DepositService},
    signer::Signer,
    wallet::leaf::WalletLeaf,
};

pub enum SparkWalletError {
    InvalidAddress(String),
    SignerError(String),
    DepositAddressUsed,
}

pub struct SparkWallet<S> {
    deposit_service: DepositService,
    network: Network,
    signer: S,
}

impl<S> SparkWallet<S>
where
    S: Signer,
{
    pub fn new(deposit_service: DepositService, network: Network, signer: S) -> Self {
        SparkWallet {
            deposit_service,
            network,
            signer,
        }
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
        let params = self.network.into();
        for (vout, output) in tx.output.iter().enumerate() {
            let Ok(address) = Address::from_script(&output.script_pubkey, &params) else {
                continue;
            };

            let Some(deposit_address) = unused_addresses.get(&address) else {
                continue;
            };

            // TODO: If leaf id is actually optional:
            //   let signingPubKey: Uint8Array;
            //   if (!depositAddress.leafId) {
            //     signingPubKey = depositAddress.userSigningPublicKey;
            //   } else {
            //     signingPubKey = await this.config.signer.generatePublicKey(
            //       sha256(depositAddress.leafId),
            //     );
            //   }

            return self.finalize_deposit(deposit_address, tx, vout).await;
        }

        Err(SparkWalletError::DepositAddressUsed)
    }

    async fn finalize_deposit(
        &self,
        address: &DepositAddress,
        tx: Transaction,
        vout: usize,
    ) -> Result<Vec<WalletLeaf>, SparkWalletError> {
        let res = self
            .deposit_service
            .create_tree_root(address, tx, vout)
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
        let signing_public_key = self.signer.generate_public_key(hash).await.map_err(|e| {
            SparkWalletError::SignerError(format!("Failed to generate public key: {}", e))
        })?;
        let address = self
            .deposit_service
            .generate_deposit_address(signing_public_key, leaf_id.to_string(), is_static)
            .await?;
        Ok(address.address)
    }
}
