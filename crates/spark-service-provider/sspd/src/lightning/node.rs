use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct DecodedInvoice {
    pub payment_hash: [u8; 32],
    pub amount_msat: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightningPaymentId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentState {
    Pending,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone)]
pub struct OutgoingPayment {
    pub state: PaymentState,
    pub preimage: Option<[u8; 32]>,
}

#[derive(Debug, Clone)]
pub struct IncomingPayment {
    pub state: PaymentState,
    /// Set once a payment has arrived for the invoice.
    pub held: Option<HeldPayment>,
}

#[derive(Debug, Clone, Copy)]
pub struct HeldPayment {
    pub amount_msat: u64,
    /// The block height at which the node fails the payment back unless it is
    /// claimed. The node keeps reporting it after it passes.
    pub claim_deadline: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum LightningNodeError {
    #[error("lightning node error: {0}")]
    Node(String),

    #[error("invoice decode error: {0}")]
    Decode(String),

    /// The node tried to send the payment and it failed. The attempt is over, and
    /// the node records the payment as failed.
    #[error("lightning payment failed to send: {0}")]
    PaymentSendingFailed(String),
}

#[derive(Debug, Clone)]
pub enum InvoiceDescription {
    Memo(String),
    Hash([u8; 32]),
}

#[async_trait]
pub trait LightningNode: Send + Sync {
    async fn decode_invoice(&self, invoice: &str) -> Result<DecodedInvoice, LightningNodeError>;

    async fn pay_invoice(
        &self,
        invoice: &str,
        amount_msat: Option<u64>,
        max_total_cltv_expiry_delta: u32,
        max_routing_fee_msat: Option<u64>,
    ) -> Result<LightningPaymentId, LightningNodeError>;

    /// The node's payment of the invoice with `payment_hash`. A payment the node
    /// reports as failed can still carry the preimage that proves it succeeded.
    async fn outgoing_payment(
        &self,
        payment_hash: &[u8; 32],
    ) -> Result<Option<OutgoingPayment>, LightningNodeError>;

    /// `amount_msat` of `None` asks for an amountless invoice.
    /// `min_final_cltv_expiry_delta` is in blocks.
    async fn create_hold_invoice(
        &self,
        payment_hash: &[u8; 32],
        amount_msat: Option<u64>,
        expiry_secs: u32,
        description: &InvoiceDescription,
        min_final_cltv_expiry_delta: u16,
    ) -> Result<String, LightningNodeError>;

    /// The hold invoices the node has reported a payment for that it may still
    /// hold. Only a hint of which invoices to read with
    /// [`LightningNode::incoming_payment`].
    fn paid_hold_invoices(&self) -> Vec<[u8; 32]>;

    /// The payment to the hold invoice for `payment_hash`: `Succeeded` once it is
    /// claimed, and `Failed` once it is cancelled.
    async fn incoming_payment(
        &self,
        payment_hash: &[u8; 32],
    ) -> Result<Option<IncomingPayment>, LightningNodeError>;

    async fn current_block_height(&self) -> Result<u32, LightningNodeError>;

    /// Returning does not mean the payment is captured, which
    /// [`LightningNode::incoming_payment`] reports. Settling a settled payment
    /// must succeed.
    async fn settle_hold_invoice(
        &self,
        payment_hash: &[u8; 32],
        preimage: &[u8; 32],
    ) -> Result<(), LightningNodeError>;

    /// Must be idempotent.
    async fn cancel_hold_invoice(&self, payment_hash: &[u8; 32]) -> Result<(), LightningNodeError>;
}

#[cfg(test)]
pub mod mock {
    use super::{
        DecodedInvoice, IncomingPayment, InvoiceDescription, LightningNode, LightningNodeError,
        LightningPaymentId, OutgoingPayment,
    };

    pub struct MockLightningNode;

    #[async_trait::async_trait]
    impl LightningNode for MockLightningNode {
        async fn decode_invoice(
            &self,
            _invoice: &str,
        ) -> Result<DecodedInvoice, LightningNodeError> {
            unimplemented!()
        }

        async fn pay_invoice(
            &self,
            _invoice: &str,
            _amount_msat: Option<u64>,
            _max_total_cltv_expiry_delta: u32,
            _max_routing_fee_msat: Option<u64>,
        ) -> Result<LightningPaymentId, LightningNodeError> {
            unimplemented!()
        }

        async fn outgoing_payment(
            &self,
            _payment_hash: &[u8; 32],
        ) -> Result<Option<OutgoingPayment>, LightningNodeError> {
            unimplemented!()
        }

        async fn create_hold_invoice(
            &self,
            payment_hash: &[u8; 32],
            _amount_msat: Option<u64>,
            _expiry_secs: u32,
            _description: &InvoiceDescription,
            _min_final_cltv_expiry_delta: u16,
        ) -> Result<String, LightningNodeError> {
            Ok(format!("lnbc-mock-{}", hex::encode(payment_hash)))
        }

        fn paid_hold_invoices(&self) -> Vec<[u8; 32]> {
            unimplemented!()
        }

        async fn incoming_payment(
            &self,
            _payment_hash: &[u8; 32],
        ) -> Result<Option<IncomingPayment>, LightningNodeError> {
            unimplemented!()
        }

        async fn current_block_height(&self) -> Result<u32, LightningNodeError> {
            unimplemented!()
        }

        async fn settle_hold_invoice(
            &self,
            _payment_hash: &[u8; 32],
            _preimage: &[u8; 32],
        ) -> Result<(), LightningNodeError> {
            unimplemented!()
        }

        async fn cancel_hold_invoice(
            &self,
            _payment_hash: &[u8; 32],
        ) -> Result<(), LightningNodeError> {
            unimplemented!()
        }
    }
}
