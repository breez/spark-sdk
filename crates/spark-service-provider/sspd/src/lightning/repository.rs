use bitcoin::OutPoint;
use bitcoin::hashes::sha256;
use bitcoin::secp256k1::PublicKey;
use spark::services::{Preimage, TransferId};

use crate::handover::HandoverReservation;

use super::node::LightningPaymentId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendPaymentStatus {
    Pending,
    Succeeded,
    /// The payment failed, or was never made.
    Failed,
}

#[derive(Debug, Clone)]
pub struct LightningSendRecord {
    pub id: String,
    pub user_identity_public_key: PublicKey,
    pub encoded_invoice: String,
    pub payment_hash: sha256::Hash,
    pub amount_sats: u64,
    pub fee_sats: u64,
    pub user_transfer_id: TransferId,
    /// Where a refund the user signed before the send pays, should they take a leaf
    /// on chain.
    pub htlc_address: String,
    pub idempotency_key: Option<String>,
    pub ln_payment_id: Option<LightningPaymentId>,
    pub preimage: Option<Preimage>,
    pub payment_status: SendPaymentStatus,
    pub leaves_claimed: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl LightningSendRecord {
    pub fn is_complete(&self) -> bool {
        match self.payment_status {
            SendPaymentStatus::Failed => true,
            SendPaymentStatus::Succeeded => self.leaves_claimed,
            SendPaymentStatus::Pending => false,
        }
    }
}

/// A confirmed, unspent output paying the HTLC of a send whose preimage the SSP holds.
#[derive(Debug, Clone)]
pub struct HtlcOutput {
    pub send_id: String,
    pub user_identity_public_key: PublicKey,
    pub preimage: Preimage,
    pub sweep_address: Option<String>,
    pub outpoint: OutPoint,
    pub value_sats: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldInvoiceStatus {
    Pending,
    Settled,
    Cancelled,
    /// The node failed the held payment back after the leaves were handed over.
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveState {
    AwaitingPayment,
    AwaitingClaimOrReturn,
    ReadyToSettle,
    Settled,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone)]
pub struct LightningReceiveRecord {
    pub id: String,
    /// The receiver.
    pub user_identity_public_key: PublicKey,
    /// The identity that asked for the invoice, which need not be the receiver.
    pub requester_identity_public_key: PublicKey,
    pub payment_hash: sha256::Hash,
    /// What the invoice asks for, `0` for an invoice that names no amount.
    pub amount_sats: u64,
    pub encoded_invoice: String,
    /// The SSP's transfer to the receiver, recorded before it is attempted.
    pub transfer_id: Option<TransferId>,
    /// What that transfer hands over: the held payment decides it for an invoice
    /// that names no amount.
    pub transfer_amount_sats: Option<u64>,
    pub reservation: Option<HandoverReservation>,
    pub preimage: Option<Preimage>,
    pub invoice_status: HoldInvoiceStatus,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    /// `None` for a description-hash invoice.
    pub memo: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl LightningReceiveRecord {
    pub fn state(&self) -> ReceiveState {
        match self.invoice_status {
            HoldInvoiceStatus::Settled => ReceiveState::Settled,
            HoldInvoiceStatus::Cancelled => ReceiveState::Cancelled,
            HoldInvoiceStatus::Failed => ReceiveState::Failed,
            HoldInvoiceStatus::Pending => {
                if self.preimage.is_some() {
                    ReceiveState::ReadyToSettle
                } else if self.transfer_id.is_some() {
                    ReceiveState::AwaitingClaimOrReturn
                } else {
                    ReceiveState::AwaitingPayment
                }
            }
        }
    }

    pub fn is_terminal(&self) -> bool {
        self.invoice_status != HoldInvoiceStatus::Pending
    }
}

#[async_trait::async_trait]
pub trait LightningStore: Send + Sync {
    async fn insert_send(&self, record: &LightningSendRecord) -> Result<(), String>;
    async fn set_send_payment_id(
        &self,
        id: &str,
        ln_payment_id: &LightningPaymentId,
    ) -> Result<(), String>;
    async fn set_send_succeeded(&self, id: &str, preimage: &Preimage) -> Result<(), String>;
    async fn set_send_failed(&self, id: &str) -> Result<(), String>;
    async fn set_send_leaves_claimed(&self, id: &str) -> Result<(), String>;
    async fn get_send(&self, id: &str) -> Result<Option<LightningSendRecord>, String>;
    async fn get_send_by_idempotency_key(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<LightningSendRecord>, String>;
    async fn get_send_by_transfer_id(
        &self,
        transfer_id: &TransferId,
    ) -> Result<Option<LightningSendRecord>, String>;
    async fn get_unfailed_send_by_payment_hash(
        &self,
        payment_hash: &sha256::Hash,
    ) -> Result<Option<LightningSendRecord>, String>;
    /// The sends that are not [`LightningSendRecord::is_complete`].
    async fn pending_sends(&self) -> Result<Vec<LightningSendRecord>, String>;
    async fn unswept_htlc_outputs(&self) -> Result<Vec<HtlcOutput>, String>;
    async fn set_htlc_sweep_address(&self, send_id: &str, address: &str) -> Result<(), String>;

    async fn insert_receive(&self, record: &LightningReceiveRecord) -> Result<(), String>;
    async fn set_receive_handover(
        &self,
        id: &str,
        transfer_id: &TransferId,
        amount_sats: u64,
        reservation: &HandoverReservation,
    ) -> Result<(), String>;
    async fn clear_receive_reservation(&self, id: &str) -> Result<(), String>;
    async fn clear_receive_handover(&self, id: &str) -> Result<(), String>;
    async fn set_receive_preimage(&self, id: &str, preimage: &Preimage) -> Result<(), String>;
    async fn set_receive_invoice_status(
        &self,
        id: &str,
        status: HoldInvoiceStatus,
    ) -> Result<(), String>;
    async fn get_receive(&self, id: &str) -> Result<Option<LightningReceiveRecord>, String>;
    async fn get_receive_by_payment_hash(
        &self,
        payment_hash: &sha256::Hash,
    ) -> Result<Option<LightningReceiveRecord>, String>;
    async fn get_receive_by_transfer_id(
        &self,
        transfer_id: &TransferId,
    ) -> Result<Option<LightningReceiveRecord>, String>;
    async fn pending_receives(&self) -> Result<Vec<LightningReceiveRecord>, String>;
    /// Pending receives without a handover whose invoice has one of `payment_hashes`.
    async fn unhanded_receives(
        &self,
        payment_hashes: &[sha256::Hash],
    ) -> Result<Vec<LightningReceiveRecord>, String>;
    /// Pending receives with a handover.
    async fn handing_over_receives(&self) -> Result<Vec<LightningReceiveRecord>, String>;
    /// At most `limit` pending receives without a handover whose invoice expired
    /// before `time`.
    async fn unhanded_receives_expired_before(
        &self,
        time: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<Vec<LightningReceiveRecord>, String>;
}

#[cfg(test)]
#[derive(Default)]
pub struct InMemoryLightningStore {
    sends: tokio::sync::RwLock<std::collections::HashMap<String, LightningSendRecord>>,
    receives: tokio::sync::RwLock<std::collections::HashMap<String, LightningReceiveRecord>>,
}

#[cfg(test)]
#[async_trait::async_trait]
impl LightningStore for InMemoryLightningStore {
    async fn insert_send(&self, record: &LightningSendRecord) -> Result<(), String> {
        let mut sends = self.sends.write().await;
        if sends.values().any(|send| {
            send.user_transfer_id == record.user_transfer_id
                || (send.payment_hash == record.payment_hash
                    && send.payment_status != SendPaymentStatus::Failed)
        }) {
            return Err("a send for this transfer or invoice exists".to_string());
        }
        sends.insert(record.id.clone(), record.clone());
        Ok(())
    }

    async fn set_send_payment_id(
        &self,
        id: &str,
        ln_payment_id: &LightningPaymentId,
    ) -> Result<(), String> {
        if let Some(record) = self.sends.write().await.get_mut(id) {
            record.ln_payment_id = Some(ln_payment_id.clone());
            record.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn set_send_succeeded(&self, id: &str, preimage: &Preimage) -> Result<(), String> {
        if let Some(record) = self.sends.write().await.get_mut(id) {
            record.preimage = Some(preimage.clone());
            record.payment_status = SendPaymentStatus::Succeeded;
            record.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn set_send_failed(&self, id: &str) -> Result<(), String> {
        if let Some(record) = self.sends.write().await.get_mut(id) {
            record.payment_status = SendPaymentStatus::Failed;
            record.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn set_send_leaves_claimed(&self, id: &str) -> Result<(), String> {
        if let Some(record) = self.sends.write().await.get_mut(id) {
            record.leaves_claimed = true;
            record.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn get_send(&self, id: &str) -> Result<Option<LightningSendRecord>, String> {
        Ok(self.sends.read().await.get(id).cloned())
    }

    async fn get_send_by_idempotency_key(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<LightningSendRecord>, String> {
        Ok(self
            .sends
            .read()
            .await
            .values()
            .find(|record| record.idempotency_key.as_deref() == Some(idempotency_key))
            .cloned())
    }

    async fn get_send_by_transfer_id(
        &self,
        transfer_id: &TransferId,
    ) -> Result<Option<LightningSendRecord>, String> {
        Ok(self
            .sends
            .read()
            .await
            .values()
            .find(|record| record.user_transfer_id == *transfer_id)
            .cloned())
    }

    async fn get_unfailed_send_by_payment_hash(
        &self,
        payment_hash: &sha256::Hash,
    ) -> Result<Option<LightningSendRecord>, String> {
        Ok(self
            .sends
            .read()
            .await
            .values()
            .find(|record| {
                record.payment_hash == *payment_hash
                    && record.payment_status != SendPaymentStatus::Failed
            })
            .cloned())
    }

    async fn pending_sends(&self) -> Result<Vec<LightningSendRecord>, String> {
        Ok(self
            .sends
            .read()
            .await
            .values()
            .filter(|record| !record.is_complete())
            .cloned()
            .collect())
    }

    async fn unswept_htlc_outputs(&self) -> Result<Vec<HtlcOutput>, String> {
        Ok(Vec::new())
    }

    async fn set_htlc_sweep_address(&self, _send_id: &str, _address: &str) -> Result<(), String> {
        Ok(())
    }

    async fn insert_receive(&self, record: &LightningReceiveRecord) -> Result<(), String> {
        self.receives
            .write()
            .await
            .insert(record.id.clone(), record.clone());
        Ok(())
    }

    async fn set_receive_handover(
        &self,
        id: &str,
        transfer_id: &TransferId,
        amount_sats: u64,
        reservation: &HandoverReservation,
    ) -> Result<(), String> {
        if let Some(record) = self.receives.write().await.get_mut(id) {
            record.transfer_id = Some(transfer_id.clone());
            record.transfer_amount_sats = Some(amount_sats);
            record.reservation = Some(reservation.clone());
            record.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn clear_receive_reservation(&self, id: &str) -> Result<(), String> {
        if let Some(record) = self.receives.write().await.get_mut(id) {
            record.reservation = None;
            record.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn clear_receive_handover(&self, id: &str) -> Result<(), String> {
        if let Some(record) = self.receives.write().await.get_mut(id) {
            record.transfer_id = None;
            record.transfer_amount_sats = None;
            record.reservation = None;
            record.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn set_receive_preimage(&self, id: &str, preimage: &Preimage) -> Result<(), String> {
        if let Some(record) = self.receives.write().await.get_mut(id) {
            record.preimage = Some(preimage.clone());
            record.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn set_receive_invoice_status(
        &self,
        id: &str,
        status: HoldInvoiceStatus,
    ) -> Result<(), String> {
        if let Some(record) = self.receives.write().await.get_mut(id) {
            record.invoice_status = status;
            record.updated_at = chrono::Utc::now();
        }
        Ok(())
    }

    async fn get_receive(&self, id: &str) -> Result<Option<LightningReceiveRecord>, String> {
        Ok(self.receives.read().await.get(id).cloned())
    }

    async fn get_receive_by_payment_hash(
        &self,
        payment_hash: &sha256::Hash,
    ) -> Result<Option<LightningReceiveRecord>, String> {
        Ok(self
            .receives
            .read()
            .await
            .values()
            .find(|record| record.payment_hash == *payment_hash)
            .cloned())
    }

    async fn get_receive_by_transfer_id(
        &self,
        transfer_id: &TransferId,
    ) -> Result<Option<LightningReceiveRecord>, String> {
        Ok(self
            .receives
            .read()
            .await
            .values()
            .find(|record| record.transfer_id.as_ref() == Some(transfer_id))
            .cloned())
    }

    async fn pending_receives(&self) -> Result<Vec<LightningReceiveRecord>, String> {
        Ok(self
            .receives
            .read()
            .await
            .values()
            .filter(|record| !record.is_terminal())
            .cloned()
            .collect())
    }

    async fn unhanded_receives(
        &self,
        payment_hashes: &[sha256::Hash],
    ) -> Result<Vec<LightningReceiveRecord>, String> {
        Ok(self
            .pending_receives()
            .await?
            .into_iter()
            .filter(|r| r.transfer_id.is_none() && payment_hashes.contains(&r.payment_hash))
            .collect())
    }

    async fn handing_over_receives(&self) -> Result<Vec<LightningReceiveRecord>, String> {
        Ok(self
            .pending_receives()
            .await?
            .into_iter()
            .filter(|r| r.transfer_id.is_some())
            .collect())
    }

    async fn unhanded_receives_expired_before(
        &self,
        time: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<Vec<LightningReceiveRecord>, String> {
        Ok(self
            .pending_receives()
            .await?
            .into_iter()
            .filter(|r| r.transfer_id.is_none() && r.expires_at < time)
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HoldInvoiceStatus, InMemoryLightningStore, LightningReceiveRecord, LightningSendRecord,
        LightningStore, ReceiveState, SendPaymentStatus,
    };
    use bitcoin::hashes::{Hash, sha256};
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use spark::services::{Preimage, TransferId};

    fn pubkey(byte: u8) -> PublicKey {
        let secp = Secp256k1::new();
        PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[byte; 32]).expect("secret"))
    }

    fn hash(byte: u8) -> sha256::Hash {
        sha256::Hash::from_byte_array([byte; 32])
    }

    fn send_record(id: &str, idempotency_key: Option<&str>) -> LightningSendRecord {
        LightningSendRecord {
            id: id.to_string(),
            user_identity_public_key: pubkey(1),
            encoded_invoice: "lnbc-test".to_string(),
            payment_hash: hash(9),
            amount_sats: 1_000,
            fee_sats: 10,
            user_transfer_id: TransferId::generate(),
            htlc_address: "bcrt1phtlc".to_string(),
            idempotency_key: idempotency_key.map(str::to_string),
            ln_payment_id: None,
            preimage: None,
            payment_status: SendPaymentStatus::Pending,
            leaves_claimed: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn send_lifecycle_and_pending() {
        let store = InMemoryLightningStore::default();
        let record = send_record("s1", Some("key-1"));
        let send_transfer_id = record.user_transfer_id.clone();
        store.insert_send(&record).await.unwrap();

        assert_eq!(store.pending_sends().await.unwrap().len(), 1);
        assert!(
            store
                .get_send_by_idempotency_key("key-1")
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(
            store
                .get_send_by_transfer_id(&send_transfer_id)
                .await
                .unwrap()
                .unwrap()
                .id,
            "s1"
        );
        assert!(
            store
                .get_send_by_transfer_id(&TransferId::generate())
                .await
                .unwrap()
                .is_none()
        );

        store
            .set_send_payment_id("s1", &super::LightningPaymentId("ln-pay-1".to_string()))
            .await
            .unwrap();
        let preimage = Preimage::try_from(vec![7u8; 32]).unwrap();
        store.set_send_succeeded("s1", &preimage).await.unwrap();
        let record = store.get_send("s1").await.unwrap().unwrap();
        assert_eq!(record.ln_payment_id.as_ref().unwrap().0, "ln-pay-1");
        assert!(record.preimage.is_some());
        assert!(!record.is_complete());
        assert_eq!(store.pending_sends().await.unwrap().len(), 1);

        store.set_send_leaves_claimed("s1").await.unwrap();
        assert!(store.get_send("s1").await.unwrap().unwrap().is_complete());
        assert!(store.pending_sends().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn receive_lifecycle_and_pending() {
        let store = InMemoryLightningStore::default();
        let record = LightningReceiveRecord {
            id: "r1".to_string(),
            user_identity_public_key: pubkey(2),
            requester_identity_public_key: pubkey(2),
            payment_hash: hash(5),
            amount_sats: 2_000,
            encoded_invoice: "lnbc-recv".to_string(),
            transfer_id: None,
            transfer_amount_sats: None,
            reservation: None,
            preimage: None,
            invoice_status: HoldInvoiceStatus::Pending,
            expires_at: chrono::Utc::now(),
            memo: Some("lunch".to_string()),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        store.insert_receive(&record).await.unwrap();
        assert_eq!(store.pending_receives().await.unwrap().len(), 1);
        assert_eq!(
            store.get_receive("r1").await.unwrap().unwrap().state(),
            ReceiveState::AwaitingPayment
        );
        assert!(
            store
                .get_receive_by_payment_hash(&hash(5))
                .await
                .unwrap()
                .is_some()
        );

        let transfer_id = TransferId::generate();
        assert!(
            store
                .get_receive_by_transfer_id(&transfer_id)
                .await
                .unwrap()
                .is_none()
        );
        let reservation = super::HandoverReservation {
            id: "reservation-1".to_string(),
            leaf_ids: Vec::new(),
        };
        store
            .set_receive_handover("r1", &transfer_id, 2_000, &reservation)
            .await
            .unwrap();
        assert_eq!(
            store.get_receive("r1").await.unwrap().unwrap().state(),
            ReceiveState::AwaitingClaimOrReturn
        );
        assert_eq!(
            store
                .get_receive_by_transfer_id(&transfer_id)
                .await
                .unwrap()
                .unwrap()
                .id,
            "r1"
        );

        let preimage = Preimage::try_from(vec![3u8; 32]).unwrap();
        store.set_receive_preimage("r1", &preimage).await.unwrap();
        assert_eq!(
            store.get_receive("r1").await.unwrap().unwrap().state(),
            ReceiveState::ReadyToSettle
        );

        store
            .set_receive_invoice_status("r1", HoldInvoiceStatus::Settled)
            .await
            .unwrap();
        let stored = store.get_receive("r1").await.unwrap().unwrap();
        assert_eq!(stored.transfer_id, Some(transfer_id));
        assert_eq!(stored.state(), ReceiveState::Settled);
        assert!(store.pending_receives().await.unwrap().is_empty());
    }
}
