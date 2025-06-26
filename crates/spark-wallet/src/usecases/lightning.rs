use std::sync::Arc;

use spark::{
    services::{LightningSendPayment, LightningService, TransferService},
    signer::Signer,
    tree::TreeNode,
};

use crate::SparkWalletError;

pub(crate) struct PayLightningInvoice<S>
where
    S: Signer,
{
    lightning_service: Arc<LightningService<S>>,
    transfer_service: Arc<TransferService<S>>,
    invoice: String,
    leaves: Vec<TreeNode>,
}

impl<S: Signer + Clone> PayLightningInvoice<S> {
    pub fn new(
        lightning_service: Arc<LightningService<S>>,
        transfer_service: Arc<TransferService<S>>,
        invoice: String,
        leaves: Vec<TreeNode>,
    ) -> Self {
        Self {
            lightning_service,
            transfer_service,
            invoice,
            leaves,
        }
    }

    pub async fn execute(&self) -> Result<LightningSendPayment, SparkWalletError> {
        let swap = self
            .lightning_service
            .start_lightning_swap(&self.invoice, &self.leaves)
            .await?;
        let _ = self
            .transfer_service
            .send_transfer_with_key_tweaks(&swap.leaves, &swap.receiver_identity_public_key)
            .await?;
        Ok(self
            .lightning_service
            .finalize_lightning_swap(&swap)
            .await?)
    }
}
