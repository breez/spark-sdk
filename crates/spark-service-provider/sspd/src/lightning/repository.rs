use bitcoin::OutPoint;
use bitcoin::hashes::sha256;
use bitcoin::secp256k1::PublicKey;
use spark::services::{Preimage, TransferId};

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
    /// The SSP's transfer to the receiver. The SSP records the transfer's id
    /// before it makes the transfer.
    pub transfer_id: Option<TransferId>,
    /// What that transfer hands over: the held payment decides it for an invoice
    /// that names no amount.
    pub transfer_amount_sats: Option<u64>,
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
    ) -> Result<(), String>;
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
pub(crate) mod scenarios {
    use std::time::Duration;

    use bitcoin::secp256k1::{Secp256k1, SecretKey};
    use bitcoin::{Address, Network};
    use chrono::{DateTime, Utc};

    use super::*;

    fn key(number: u8) -> PublicKey {
        SecretKey::from_slice(&[number; 32])
            .unwrap()
            .public_key(&Secp256k1::new())
    }

    pub(crate) fn preimage(number: u8) -> Preimage {
        Preimage::try_from(vec![number; 32]).unwrap()
    }

    fn payment_hash(number: u8) -> sha256::Hash {
        preimage(number).compute_hash()
    }

    fn payment_id(number: u8) -> LightningPaymentId {
        LightningPaymentId(format!("payment-{number}"))
    }

    fn time(seconds: u32) -> DateTime<Utc> {
        DateTime::from_timestamp(i64::from(seconds), 0).unwrap()
    }

    /// Sends of different numbers share no value that a store requires to be
    /// unique.
    pub(crate) fn new_send(number: u8) -> LightningSendRecord {
        let htlc_address = Address::p2tr(
            &Secp256k1::new(),
            key(number).x_only_public_key().0,
            None,
            Network::Regtest,
        );
        LightningSendRecord {
            id: format!("send-{number}"),
            user_identity_public_key: key(number),
            encoded_invoice: format!("lnbcrt-send-{number}"),
            payment_hash: payment_hash(number),
            amount_sats: 1_000,
            fee_sats: 10,
            user_transfer_id: TransferId::from_name(&format!("send-{number}")),
            htlc_address: htlc_address.to_string(),
            idempotency_key: None,
            ln_payment_id: None,
            preimage: None,
            payment_status: SendPaymentStatus::Pending,
            leaves_claimed: false,
            created_at: time(u32::from(number)),
            updated_at: time(u32::from(number)),
        }
    }

    /// Receives of different numbers share no value that a store requires to be
    /// unique.
    fn new_receive(number: u8) -> LightningReceiveRecord {
        LightningReceiveRecord {
            id: format!("receive-{number}"),
            user_identity_public_key: key(number),
            requester_identity_public_key: key(number),
            payment_hash: payment_hash(number),
            amount_sats: 2_000,
            encoded_invoice: format!("lnbcrt-receive-{number}"),
            transfer_id: None,
            transfer_amount_sats: None,
            preimage: None,
            invoice_status: HoldInvoiceStatus::Pending,
            expires_at: time(3_600),
            memo: Some(format!("receive {number}")),
            created_at: time(u32::from(number)),
            updated_at: time(u32::from(number)),
        }
    }

    async fn stored_send(store: &dyn LightningStore, number: u8) -> LightningSendRecord {
        let send = new_send(number);
        store.insert_send(&send).await.unwrap();
        send
    }

    async fn stored_receive(store: &dyn LightningStore, number: u8) -> LightningReceiveRecord {
        let receive = new_receive(number);
        store.insert_receive(&receive).await.unwrap();
        receive
    }

    async fn read_send(store: &dyn LightningStore, id: &str) -> LightningSendRecord {
        store.get_send(id).await.unwrap().unwrap()
    }

    async fn read_receive(store: &dyn LightningStore, id: &str) -> LightningReceiveRecord {
        store.get_receive(id).await.unwrap().unwrap()
    }

    fn handover_transfer(receive: &LightningReceiveRecord) -> TransferId {
        TransferId::from_name(&receive.id)
    }

    async fn hand_over(store: &dyn LightningStore, receive: &LightningReceiveRecord) {
        store
            .set_receive_handover(&receive.id, &handover_transfer(receive), 2_000)
            .await
            .unwrap();
    }

    /// The records have no equality, so a scenario compares them by how they
    /// print.
    fn fields(record: &impl std::fmt::Debug) -> String {
        format!("{record:?}")
    }

    /// The function sorts the ids because a store returns some lists in no set
    /// order.
    fn send_ids(sends: &[LightningSendRecord]) -> Vec<&str> {
        let mut ids: Vec<&str> = sends.iter().map(|send| send.id.as_str()).collect();
        ids.sort_unstable();
        ids
    }

    fn receive_ids(receives: &[LightningReceiveRecord]) -> Vec<&str> {
        let mut ids: Vec<&str> = receives.iter().map(|receive| receive.id.as_str()).collect();
        ids.sort_unstable();
        ids
    }

    /// Waits long enough for the store's clock to show a later time.
    async fn a_moment() {
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    struct Sends {
        new: LightningSendRecord,
        paying: LightningSendRecord,
        paid: LightningSendRecord,
        complete: LightningSendRecord,
        failed: LightningSendRecord,
    }

    async fn sends_in_every_state(store: &dyn LightningStore) -> Sends {
        let stored = async |id: &str, number: u8| {
            let send = LightningSendRecord {
                id: id.to_string(),
                ..new_send(number)
            };
            store.insert_send(&send).await.unwrap();
            send
        };
        let sends = Sends {
            new: stored("new", 1).await,
            paying: stored("paying", 2).await,
            paid: stored("paid", 3).await,
            complete: stored("complete", 4).await,
            failed: stored("failed", 5).await,
        };
        for (send, number) in [
            (&sends.paying, 2),
            (&sends.paid, 3),
            (&sends.complete, 4),
            (&sends.failed, 5),
        ] {
            store
                .set_send_payment_id(&send.id, &payment_id(number))
                .await
                .unwrap();
        }
        for (send, number) in [(&sends.paid, 3), (&sends.complete, 4)] {
            store
                .set_send_succeeded(&send.id, &preimage(number))
                .await
                .unwrap();
        }
        store
            .set_send_leaves_claimed(&sends.complete.id)
            .await
            .unwrap();
        store.set_send_failed(&sends.failed.id).await.unwrap();
        sends
    }

    struct Receives {
        awaiting_payment: LightningReceiveRecord,
        handed_over: LightningReceiveRecord,
        handover_cleared: LightningReceiveRecord,
        ready_to_settle: LightningReceiveRecord,
        settled: LightningReceiveRecord,
        cancelled: LightningReceiveRecord,
        failed: LightningReceiveRecord,
    }

    impl Receives {
        fn all(&self) -> [&LightningReceiveRecord; 7] {
            [
                &self.awaiting_payment,
                &self.handed_over,
                &self.handover_cleared,
                &self.ready_to_settle,
                &self.settled,
                &self.cancelled,
                &self.failed,
            ]
        }
    }

    async fn receives_in_every_state(store: &dyn LightningStore) -> Receives {
        let stored = async |id: &str, number: u8| {
            let receive = LightningReceiveRecord {
                id: id.to_string(),
                ..new_receive(number)
            };
            store.insert_receive(&receive).await.unwrap();
            receive
        };
        let receives = Receives {
            awaiting_payment: stored("awaiting-payment", 1).await,
            handed_over: stored("handed-over", 2).await,
            handover_cleared: stored("handover-cleared", 3).await,
            ready_to_settle: stored("ready-to-settle", 4).await,
            settled: stored("settled", 5).await,
            cancelled: stored("cancelled", 6).await,
            failed: stored("failed", 7).await,
        };
        for receive in [
            &receives.handed_over,
            &receives.handover_cleared,
            &receives.ready_to_settle,
            &receives.settled,
            &receives.failed,
        ] {
            hand_over(store, receive).await;
        }
        store
            .clear_receive_handover(&receives.handover_cleared.id)
            .await
            .unwrap();
        for (receive, number) in [
            (&receives.ready_to_settle, 4),
            (&receives.settled, 5),
            (&receives.failed, 7),
        ] {
            store
                .set_receive_preimage(&receive.id, &preimage(number))
                .await
                .unwrap();
        }
        for (receive, status) in [
            (&receives.settled, HoldInvoiceStatus::Settled),
            (&receives.cancelled, HoldInvoiceStatus::Cancelled),
            (&receives.failed, HoldInvoiceStatus::Failed),
        ] {
            store
                .set_receive_invoice_status(&receive.id, status)
                .await
                .unwrap();
        }
        receives
    }

    pub(crate) async fn a_send_reads_back_as_it_was_stored(store: &dyn LightningStore) {
        let send = LightningSendRecord {
            idempotency_key: Some("key-1".to_string()),
            ..new_send(1)
        };
        store.insert_send(&send).await.unwrap();

        let stored = read_send(store, &send.id).await;

        assert_eq!(stored.id, send.id);
        assert_eq!(
            stored.user_identity_public_key,
            send.user_identity_public_key
        );
        assert_eq!(stored.encoded_invoice, send.encoded_invoice);
        assert_eq!(stored.payment_hash, send.payment_hash);
        assert_eq!(stored.amount_sats, 1_000);
        assert_eq!(stored.fee_sats, 10);
        assert_eq!(stored.user_transfer_id, send.user_transfer_id);
        assert_eq!(stored.htlc_address, send.htlc_address);
        assert_eq!(stored.idempotency_key, send.idempotency_key);
    }

    pub(crate) async fn a_new_send_is_pending_without_a_payment(store: &dyn LightningStore) {
        let send = stored_send(store, 1).await;

        let stored = read_send(store, &send.id).await;

        assert_eq!(stored.payment_status, SendPaymentStatus::Pending);
        assert_eq!(stored.ln_payment_id, None);
        assert_eq!(stored.preimage, None);
        assert!(!stored.leaves_claimed);
    }

    pub(crate) async fn the_store_sets_the_state_and_times_of_a_new_send(
        store: &dyn LightningStore,
    ) {
        let send = LightningSendRecord {
            ln_payment_id: Some(payment_id(1)),
            preimage: Some(preimage(1)),
            payment_status: SendPaymentStatus::Succeeded,
            leaves_claimed: true,
            ..new_send(1)
        };
        store.insert_send(&send).await.unwrap();

        let stored = read_send(store, &send.id).await;

        assert_eq!(stored.payment_status, SendPaymentStatus::Pending);
        assert_eq!(stored.ln_payment_id, None);
        assert_eq!(stored.preimage, None);
        assert!(!stored.leaves_claimed);
        assert_ne!(stored.created_at, send.created_at);
        assert_ne!(stored.updated_at, send.updated_at);
    }

    pub(crate) async fn a_send_is_found_by_its_idempotency_key(store: &dyn LightningStore) {
        for (number, key) in [(1, "key-1"), (2, "key-2")] {
            let send = LightningSendRecord {
                idempotency_key: Some(key.to_string()),
                ..new_send(number)
            };
            store.insert_send(&send).await.unwrap();
        }
        stored_send(store, 3).await;
        let found = async |key: &str| {
            let send = store.get_send_by_idempotency_key(key).await.unwrap();
            send.map(|send| send.id)
        };

        assert_eq!(found("key-1").await.as_deref(), Some("send-1"));
        assert_eq!(found("key-2").await.as_deref(), Some("send-2"));
        assert_eq!(found("key-3").await, None);
    }

    pub(crate) async fn a_send_is_found_by_the_transfer_that_paid_for_it(
        store: &dyn LightningStore,
    ) {
        let (first, second) = (stored_send(store, 1).await, stored_send(store, 2).await);
        let found = async |transfer_id: &TransferId| {
            let send = store.get_send_by_transfer_id(transfer_id).await.unwrap();
            send.map(|send| send.id)
        };

        assert_eq!(
            found(&first.user_transfer_id).await.as_deref(),
            Some("send-1")
        );
        assert_eq!(
            found(&second.user_transfer_id).await.as_deref(),
            Some("send-2")
        );
        assert_eq!(found(&new_send(3).user_transfer_id).await, None);
    }

    pub(crate) async fn a_send_is_found_by_its_payment_hash_unless_it_failed(
        store: &dyn LightningStore,
    ) {
        let sends = sends_in_every_state(store).await;
        let found = async |send: &LightningSendRecord| {
            let found = store
                .get_unfailed_send_by_payment_hash(&send.payment_hash)
                .await
                .unwrap();
            found.map(|found| found.id)
        };

        for send in [&sends.new, &sends.paying, &sends.paid, &sends.complete] {
            assert_eq!(found(send).await.as_ref(), Some(&send.id));
        }
        assert_eq!(found(&sends.failed).await, None);
        assert_eq!(found(&new_send(6)).await, None);
    }

    pub(crate) async fn a_send_reads_back_the_payment_made_for_it(store: &dyn LightningStore) {
        let send = stored_send(store, 1).await;

        store
            .set_send_payment_id(&send.id, &payment_id(1))
            .await
            .unwrap();

        let stored = read_send(store, &send.id).await;
        assert_eq!(stored.ln_payment_id, Some(payment_id(1)));
        assert_eq!(stored.payment_status, SendPaymentStatus::Pending);
    }

    pub(crate) async fn a_paid_send_reads_back_its_preimage(store: &dyn LightningStore) {
        let send = stored_send(store, 1).await;
        store
            .set_send_payment_id(&send.id, &payment_id(1))
            .await
            .unwrap();

        store
            .set_send_succeeded(&send.id, &preimage(1))
            .await
            .unwrap();

        let stored = read_send(store, &send.id).await;
        assert_eq!(stored.preimage, Some(preimage(1)));
        assert_eq!(stored.payment_status, SendPaymentStatus::Succeeded);
        assert_eq!(stored.ln_payment_id, Some(payment_id(1)));
        assert!(!stored.leaves_claimed);
    }

    pub(crate) async fn a_paid_send_is_complete_once_its_leaves_are_claimed(
        store: &dyn LightningStore,
    ) {
        let send = stored_send(store, 1).await;
        store
            .set_send_succeeded(&send.id, &preimage(1))
            .await
            .unwrap();
        assert!(!read_send(store, &send.id).await.is_complete());

        store.set_send_leaves_claimed(&send.id).await.unwrap();

        let stored = read_send(store, &send.id).await;
        assert!(stored.leaves_claimed);
        assert!(stored.is_complete());
    }

    pub(crate) async fn a_failed_send_is_complete(store: &dyn LightningStore) {
        let send = stored_send(store, 1).await;

        store.set_send_failed(&send.id).await.unwrap();

        let stored = read_send(store, &send.id).await;
        assert_eq!(stored.payment_status, SendPaymentStatus::Failed);
        assert!(stored.is_complete());
    }

    pub(crate) async fn the_pending_sends_are_those_that_are_not_complete(
        store: &dyn LightningStore,
    ) {
        sends_in_every_state(store).await;

        let pending = store.pending_sends().await.unwrap();

        assert_eq!(send_ids(&pending), ["new", "paid", "paying"]);
    }

    pub(crate) async fn pending_sends_are_listed_in_order_of_creation(store: &dyn LightningStore) {
        let first = stored_send(store, 1).await;
        stored_send(store, 2).await;
        stored_send(store, 3).await;
        // After this write, the send created first is also the send written last.
        store
            .set_send_payment_id(&first.id, &payment_id(1))
            .await
            .unwrap();

        let pending = store.pending_sends().await.unwrap();

        assert_eq!(pending.len(), 3);
        assert!(pending.is_sorted_by_key(|send| send.created_at));
    }

    pub(crate) async fn a_second_send_with_a_stored_id_is_refused(store: &dyn LightningStore) {
        let first = stored_send(store, 1).await;
        let second = LightningSendRecord {
            id: first.id.clone(),
            ..new_send(2)
        };

        assert!(store.insert_send(&second).await.is_err());

        let stored = read_send(store, &first.id).await;
        assert_eq!(stored.user_transfer_id, first.user_transfer_id);
    }

    pub(crate) async fn a_second_send_for_a_transfer_is_refused(store: &dyn LightningStore) {
        let first = stored_send(store, 1).await;
        let second = LightningSendRecord {
            user_transfer_id: first.user_transfer_id.clone(),
            ..new_send(2)
        };

        assert!(store.insert_send(&second).await.is_err());

        assert!(store.get_send(&second.id).await.unwrap().is_none());
    }

    pub(crate) async fn a_second_send_with_a_stored_idempotency_key_is_refused(
        store: &dyn LightningStore,
    ) {
        let first = LightningSendRecord {
            idempotency_key: Some("key-1".to_string()),
            ..new_send(1)
        };
        store.insert_send(&first).await.unwrap();
        let second = LightningSendRecord {
            idempotency_key: first.idempotency_key.clone(),
            ..new_send(2)
        };

        assert!(store.insert_send(&second).await.is_err());

        assert!(store.get_send(&second.id).await.unwrap().is_none());
    }

    pub(crate) async fn sends_without_an_idempotency_key_do_not_collide(
        store: &dyn LightningStore,
    ) {
        let (first, second) = (stored_send(store, 1).await, stored_send(store, 2).await);

        assert_eq!(read_send(store, &first.id).await.idempotency_key, None);
        assert_eq!(read_send(store, &second.id).await.idempotency_key, None);
    }

    pub(crate) async fn a_second_send_of_an_invoice_is_refused_while_the_first_has_not_failed(
        store: &dyn LightningStore,
    ) {
        let sends = sends_in_every_state(store).await;

        for first in [&sends.new, &sends.paying, &sends.paid, &sends.complete] {
            let second = LightningSendRecord {
                payment_hash: first.payment_hash,
                ..new_send(6)
            };
            assert!(store.insert_send(&second).await.is_err(), "{}", first.id);
        }
        assert!(store.get_send("send-6").await.unwrap().is_none());
    }

    pub(crate) async fn an_invoice_can_be_sent_again_after_its_send_failed(
        store: &dyn LightningStore,
    ) {
        let first = stored_send(store, 1).await;
        store.set_send_failed(&first.id).await.unwrap();
        let second = LightningSendRecord {
            payment_hash: first.payment_hash,
            ..new_send(2)
        };

        store.insert_send(&second).await.unwrap();

        let unfailed = store
            .get_unfailed_send_by_payment_hash(&first.payment_hash)
            .await
            .unwrap();
        assert_eq!(unfailed.map(|send| send.id), Some(second.id));
        let failed = read_send(store, &first.id).await;
        assert_eq!(failed.payment_status, SendPaymentStatus::Failed);
    }

    pub(crate) async fn a_write_to_one_send_leaves_another_as_it_was(store: &dyn LightningStore) {
        let (paid, failed) = (stored_send(store, 1).await, stored_send(store, 2).await);
        let other = stored_send(store, 3).await;
        let before = read_send(store, &other.id).await;

        store
            .set_send_payment_id(&paid.id, &payment_id(1))
            .await
            .unwrap();
        store
            .set_send_succeeded(&paid.id, &preimage(1))
            .await
            .unwrap();
        store.set_send_leaves_claimed(&paid.id).await.unwrap();
        store.set_send_failed(&failed.id).await.unwrap();

        let after = read_send(store, &other.id).await;
        assert_eq!(fields(&after), fields(&before));
    }

    pub(crate) async fn a_write_to_a_send_the_store_does_not_have_stores_nothing(
        store: &dyn LightningStore,
    ) {
        stored_send(store, 1).await;

        store
            .set_send_payment_id("send-2", &payment_id(2))
            .await
            .unwrap();
        store
            .set_send_succeeded("send-2", &preimage(2))
            .await
            .unwrap();
        store.set_send_leaves_claimed("send-2").await.unwrap();
        store.set_send_failed("send-2").await.unwrap();
        store
            .set_htlc_sweep_address("send-2", "bcrt1p-sweep")
            .await
            .unwrap();

        assert!(store.get_send("send-2").await.unwrap().is_none());
        let pending = store.pending_sends().await.unwrap();
        assert_eq!(send_ids(&pending), ["send-1"]);
    }

    pub(crate) async fn each_write_to_a_send_moves_its_update_time_forward(
        store: &dyn LightningStore,
    ) {
        let (paid, failed) = (stored_send(store, 1).await, stored_send(store, 2).await);
        let updated_at =
            async |send: &LightningSendRecord| read_send(store, &send.id).await.updated_at;
        let mut paid_times = vec![updated_at(&paid).await];
        let failed_stored_at = updated_at(&failed).await;

        a_moment().await;
        store
            .set_send_payment_id(&paid.id, &payment_id(1))
            .await
            .unwrap();
        paid_times.push(updated_at(&paid).await);
        a_moment().await;
        store
            .set_send_succeeded(&paid.id, &preimage(1))
            .await
            .unwrap();
        paid_times.push(updated_at(&paid).await);
        a_moment().await;
        store.set_send_leaves_claimed(&paid.id).await.unwrap();
        paid_times.push(updated_at(&paid).await);
        store.set_send_failed(&failed.id).await.unwrap();

        assert!(paid_times.is_sorted_by(|earlier, later| earlier < later));
        assert!(failed_stored_at < updated_at(&failed).await);
    }

    pub(crate) async fn a_paid_send_with_nothing_on_chain_has_no_htlc_output_to_sweep(
        store: &dyn LightningStore,
    ) {
        let send = stored_send(store, 1).await;
        store
            .set_send_succeeded(&send.id, &preimage(1))
            .await
            .unwrap();

        let outputs = store.unswept_htlc_outputs().await.unwrap();

        assert!(outputs.is_empty());
    }

    pub(crate) async fn a_send_and_a_receive_can_have_the_same_payment_hash(
        store: &dyn LightningStore,
    ) {
        let receive = stored_receive(store, 1).await;
        let send = LightningSendRecord {
            payment_hash: receive.payment_hash,
            ..new_send(2)
        };

        store.insert_send(&send).await.unwrap();

        let found_send = store
            .get_unfailed_send_by_payment_hash(&receive.payment_hash)
            .await
            .unwrap();
        let found_receive = store
            .get_receive_by_payment_hash(&receive.payment_hash)
            .await
            .unwrap();
        assert_eq!(found_send.map(|send| send.id), Some(send.id));
        assert_eq!(found_receive.map(|receive| receive.id), Some(receive.id));
    }

    pub(crate) async fn a_receive_reads_back_as_it_was_stored(store: &dyn LightningStore) {
        let receive = LightningReceiveRecord {
            requester_identity_public_key: key(2),
            expires_at: DateTime::from_timestamp(3_600, 123_456_000).unwrap(),
            ..new_receive(1)
        };
        store.insert_receive(&receive).await.unwrap();

        let stored = read_receive(store, &receive.id).await;

        assert_eq!(stored.id, receive.id);
        assert_eq!(stored.user_identity_public_key, key(1));
        assert_eq!(stored.requester_identity_public_key, key(2));
        assert_eq!(stored.payment_hash, receive.payment_hash);
        assert_eq!(stored.amount_sats, 2_000);
        assert_eq!(stored.encoded_invoice, receive.encoded_invoice);
        assert_eq!(stored.expires_at, receive.expires_at);
        assert_eq!(stored.memo, receive.memo);
    }

    pub(crate) async fn a_receive_without_a_memo_reads_back_without_one(
        store: &dyn LightningStore,
    ) {
        let receive = LightningReceiveRecord {
            memo: None,
            ..new_receive(1)
        };
        store.insert_receive(&receive).await.unwrap();

        let stored = read_receive(store, &receive.id).await;

        assert_eq!(stored.memo, None);
    }

    pub(crate) async fn a_new_receive_awaits_payment(store: &dyn LightningStore) {
        let receive = stored_receive(store, 1).await;

        let stored = read_receive(store, &receive.id).await;

        assert_eq!(stored.invoice_status, HoldInvoiceStatus::Pending);
        assert_eq!(stored.transfer_id, None);
        assert_eq!(stored.transfer_amount_sats, None);
        assert_eq!(stored.preimage, None);
        assert_eq!(stored.state(), ReceiveState::AwaitingPayment);
    }

    pub(crate) async fn the_store_sets_the_state_and_times_of_a_new_receive(
        store: &dyn LightningStore,
    ) {
        let receive = LightningReceiveRecord {
            transfer_id: Some(TransferId::generate()),
            transfer_amount_sats: Some(2_000),
            preimage: Some(preimage(1)),
            invoice_status: HoldInvoiceStatus::Settled,
            ..new_receive(1)
        };
        store.insert_receive(&receive).await.unwrap();

        let stored = read_receive(store, &receive.id).await;

        assert_eq!(stored.invoice_status, HoldInvoiceStatus::Pending);
        assert_eq!(stored.transfer_id, None);
        assert_eq!(stored.transfer_amount_sats, None);
        assert_eq!(stored.preimage, None);
        assert_ne!(stored.created_at, receive.created_at);
        assert_ne!(stored.updated_at, receive.updated_at);
    }

    pub(crate) async fn a_receive_is_found_by_its_payment_hash(store: &dyn LightningStore) {
        let (first, second) = (
            stored_receive(store, 1).await,
            stored_receive(store, 2).await,
        );
        let found = async |payment_hash: &sha256::Hash| {
            let receive = store
                .get_receive_by_payment_hash(payment_hash)
                .await
                .unwrap();
            receive.map(|receive| receive.id)
        };

        assert_eq!(
            found(&first.payment_hash).await.as_deref(),
            Some("receive-1")
        );
        assert_eq!(
            found(&second.payment_hash).await.as_deref(),
            Some("receive-2")
        );
        assert_eq!(found(&payment_hash(3)).await, None);
    }

    pub(crate) async fn a_receive_is_found_by_its_payment_hash_in_every_state(
        store: &dyn LightningStore,
    ) {
        let receives = receives_in_every_state(store).await;

        for receive in receives.all() {
            let found = store
                .get_receive_by_payment_hash(&receive.payment_hash)
                .await
                .unwrap();
            assert_eq!(found.map(|found| found.id).as_ref(), Some(&receive.id));
        }
    }

    pub(crate) async fn a_second_receive_with_a_stored_id_is_refused(store: &dyn LightningStore) {
        let first = stored_receive(store, 1).await;
        let second = LightningReceiveRecord {
            id: first.id.clone(),
            ..new_receive(2)
        };

        assert!(store.insert_receive(&second).await.is_err());

        let stored = read_receive(store, &first.id).await;
        assert_eq!(stored.payment_hash, first.payment_hash);
    }

    pub(crate) async fn a_second_receive_for_a_payment_hash_is_always_refused(
        store: &dyn LightningStore,
    ) {
        let receives = receives_in_every_state(store).await;

        for first in receives.all() {
            let second = LightningReceiveRecord {
                payment_hash: first.payment_hash,
                ..new_receive(8)
            };
            assert!(store.insert_receive(&second).await.is_err(), "{}", first.id);
        }
        assert!(store.get_receive("receive-8").await.unwrap().is_none());
    }

    pub(crate) async fn a_receive_reads_back_its_handover(store: &dyn LightningStore) {
        let receive = LightningReceiveRecord {
            amount_sats: 0,
            ..new_receive(1)
        };
        store.insert_receive(&receive).await.unwrap();
        let transfer_id = handover_transfer(&receive);

        store
            .set_receive_handover(&receive.id, &transfer_id, 1_500)
            .await
            .unwrap();

        let stored = read_receive(store, &receive.id).await;
        assert_eq!(stored.transfer_id, Some(transfer_id));
        assert_eq!(stored.transfer_amount_sats, Some(1_500));
        assert_eq!(stored.amount_sats, 0);
        assert_eq!(stored.state(), ReceiveState::AwaitingClaimOrReturn);
    }

    pub(crate) async fn a_cleared_handover_leaves_the_receive_awaiting_payment(
        store: &dyn LightningStore,
    ) {
        let receive = stored_receive(store, 1).await;
        hand_over(store, &receive).await;

        store.clear_receive_handover(&receive.id).await.unwrap();

        let stored = read_receive(store, &receive.id).await;
        assert_eq!(stored.transfer_id, None);
        assert_eq!(stored.transfer_amount_sats, None);
        assert_eq!(stored.state(), ReceiveState::AwaitingPayment);
    }

    pub(crate) async fn a_receive_with_a_preimage_is_ready_to_settle(store: &dyn LightningStore) {
        let receive = stored_receive(store, 1).await;
        hand_over(store, &receive).await;

        store
            .set_receive_preimage(&receive.id, &preimage(1))
            .await
            .unwrap();

        let stored = read_receive(store, &receive.id).await;
        assert_eq!(stored.preimage, Some(preimage(1)));
        assert_eq!(stored.state(), ReceiveState::ReadyToSettle);
    }

    pub(crate) async fn a_receive_reads_back_how_it_ended(store: &dyn LightningStore) {
        let ends = [
            (1, HoldInvoiceStatus::Settled, ReceiveState::Settled),
            (2, HoldInvoiceStatus::Cancelled, ReceiveState::Cancelled),
            (3, HoldInvoiceStatus::Failed, ReceiveState::Failed),
        ];
        for (number, status, state) in ends {
            let receive = stored_receive(store, number).await;

            store
                .set_receive_invoice_status(&receive.id, status)
                .await
                .unwrap();

            let stored = read_receive(store, &receive.id).await;
            assert_eq!(stored.invoice_status, status);
            assert_eq!(stored.state(), state);
        }
    }

    pub(crate) async fn a_settled_receive_keeps_its_handover_and_preimage(
        store: &dyn LightningStore,
    ) {
        let receive = stored_receive(store, 1).await;
        hand_over(store, &receive).await;
        store
            .set_receive_preimage(&receive.id, &preimage(1))
            .await
            .unwrap();

        store
            .set_receive_invoice_status(&receive.id, HoldInvoiceStatus::Settled)
            .await
            .unwrap();

        let stored = read_receive(store, &receive.id).await;
        assert_eq!(stored.transfer_id, Some(handover_transfer(&receive)));
        assert_eq!(stored.transfer_amount_sats, Some(2_000));
        assert_eq!(stored.preimage, Some(preimage(1)));
    }

    pub(crate) async fn the_pending_receives_are_those_whose_invoice_is_pending(
        store: &dyn LightningStore,
    ) {
        receives_in_every_state(store).await;

        let pending = store.pending_receives().await.unwrap();

        assert_eq!(
            receive_ids(&pending),
            [
                "awaiting-payment",
                "handed-over",
                "handover-cleared",
                "ready-to-settle"
            ]
        );
    }

    pub(crate) async fn pending_receives_are_listed_in_order_of_creation(
        store: &dyn LightningStore,
    ) {
        let first = stored_receive(store, 1).await;
        stored_receive(store, 2).await;
        stored_receive(store, 3).await;
        // After this write, the receive created first is also the receive
        // written last.
        hand_over(store, &first).await;

        let pending = store.pending_receives().await.unwrap();

        assert_eq!(pending.len(), 3);
        assert!(pending.is_sorted_by_key(|receive| receive.created_at));
    }

    pub(crate) async fn the_receives_handing_over_are_the_pending_ones_with_a_handover(
        store: &dyn LightningStore,
    ) {
        receives_in_every_state(store).await;

        let handing_over = store.handing_over_receives().await.unwrap();

        assert_eq!(
            receive_ids(&handing_over),
            ["handed-over", "ready-to-settle"]
        );
    }

    pub(crate) async fn receives_handing_over_are_listed_in_order_of_creation(
        store: &dyn LightningStore,
    ) {
        let first = stored_receive(store, 1).await;
        let second = stored_receive(store, 2).await;
        let third = stored_receive(store, 3).await;
        for receive in [&third, &first, &second] {
            hand_over(store, receive).await;
        }

        let handing_over = store.handing_over_receives().await.unwrap();

        assert_eq!(handing_over.len(), 3);
        assert!(handing_over.is_sorted_by_key(|receive| receive.created_at));
    }

    pub(crate) async fn no_payment_hashes_list_no_unhanded_receives(store: &dyn LightningStore) {
        stored_receive(store, 1).await;

        let listed = store.unhanded_receives(&[]).await.unwrap();

        assert!(listed.is_empty());
    }

    pub(crate) async fn one_payment_hash_lists_its_unhanded_receive(store: &dyn LightningStore) {
        let receive = stored_receive(store, 1).await;
        stored_receive(store, 2).await;

        let listed = store
            .unhanded_receives(&[receive.payment_hash])
            .await
            .unwrap();

        assert_eq!(receive_ids(&listed), ["receive-1"]);
    }

    pub(crate) async fn several_payment_hashes_list_each_of_their_unhanded_receives(
        store: &dyn LightningStore,
    ) {
        let first = stored_receive(store, 1).await;
        stored_receive(store, 2).await;
        let third = stored_receive(store, 3).await;

        let listed = store
            .unhanded_receives(&[first.payment_hash, third.payment_hash])
            .await
            .unwrap();

        assert_eq!(receive_ids(&listed), ["receive-1", "receive-3"]);
    }

    pub(crate) async fn a_payment_hash_without_a_receive_lists_nothing(store: &dyn LightningStore) {
        let receive = stored_receive(store, 1).await;

        let alone = store.unhanded_receives(&[payment_hash(2)]).await.unwrap();
        let among = store
            .unhanded_receives(&[payment_hash(2), receive.payment_hash])
            .await
            .unwrap();

        assert!(alone.is_empty());
        assert_eq!(receive_ids(&among), ["receive-1"]);
    }

    pub(crate) async fn a_repeated_payment_hash_lists_its_receive_once(store: &dyn LightningStore) {
        let receive = stored_receive(store, 1).await;

        let listed = store
            .unhanded_receives(&[receive.payment_hash, receive.payment_hash])
            .await
            .unwrap();

        assert_eq!(receive_ids(&listed), ["receive-1"]);
    }

    pub(crate) async fn only_pending_receives_without_a_handover_are_listed_as_unhanded(
        store: &dyn LightningStore,
    ) {
        let receives = receives_in_every_state(store).await;
        let payment_hashes = receives.all().map(|receive| receive.payment_hash);

        let unhanded = store.unhanded_receives(&payment_hashes).await.unwrap();

        assert_eq!(
            receive_ids(&unhanded),
            ["awaiting-payment", "handover-cleared"]
        );
    }

    pub(crate) async fn only_a_receive_that_expired_before_the_time_is_listed(
        store: &dyn LightningStore,
    ) {
        for (number, expires_at) in [(1, time(100)), (2, time(200)), (3, time(300))] {
            let receive = LightningReceiveRecord {
                expires_at,
                ..new_receive(number)
            };
            store.insert_receive(&receive).await.unwrap();
        }

        let expired = store
            .unhanded_receives_expired_before(time(200), 10)
            .await
            .unwrap();

        assert_eq!(receive_ids(&expired), ["receive-1"]);
    }

    pub(crate) async fn only_pending_receives_without_a_handover_are_listed_as_expired(
        store: &dyn LightningStore,
    ) {
        receives_in_every_state(store).await;

        let expired = store
            .unhanded_receives_expired_before(time(3_601), 10)
            .await
            .unwrap();

        assert_eq!(
            receive_ids(&expired),
            ["awaiting-payment", "handover-cleared"]
        );
    }

    pub(crate) async fn no_more_expired_receives_are_listed_than_the_limit(
        store: &dyn LightningStore,
    ) {
        for number in 1..=3 {
            stored_receive(store, number).await;
        }
        let listed = async |limit: i64| {
            let expired = store
                .unhanded_receives_expired_before(time(3_601), limit)
                .await
                .unwrap();
            expired.len()
        };

        assert_eq!(listed(0).await, 0);
        assert_eq!(listed(2).await, 2);
        assert_eq!(listed(3).await, 3);
        assert_eq!(listed(4).await, 3);
    }

    async fn receives_expiring_out_of_order(store: &dyn LightningStore) {
        for (number, expires_at) in [(1, time(300)), (2, time(100)), (3, time(200))] {
            let receive = LightningReceiveRecord {
                expires_at,
                ..new_receive(number)
            };
            store.insert_receive(&receive).await.unwrap();
        }
    }

    pub(crate) async fn expired_receives_are_listed_in_order_of_expiry(store: &dyn LightningStore) {
        receives_expiring_out_of_order(store).await;

        let expired = store
            .unhanded_receives_expired_before(time(400), 10)
            .await
            .unwrap();

        let ids: Vec<&str> = expired.iter().map(|receive| receive.id.as_str()).collect();
        assert_eq!(ids, ["receive-2", "receive-3", "receive-1"]);
    }

    pub(crate) async fn a_limit_keeps_the_receives_that_expired_first(store: &dyn LightningStore) {
        receives_expiring_out_of_order(store).await;

        let expired = store
            .unhanded_receives_expired_before(time(400), 2)
            .await
            .unwrap();

        assert_eq!(receive_ids(&expired), ["receive-2", "receive-3"]);
    }

    pub(crate) async fn a_write_to_one_receive_leaves_another_as_it_was(
        store: &dyn LightningStore,
    ) {
        let (settled, returned) = (
            stored_receive(store, 1).await,
            stored_receive(store, 2).await,
        );
        let other = stored_receive(store, 3).await;
        hand_over(store, &other).await;
        let before = read_receive(store, &other.id).await;

        hand_over(store, &settled).await;
        store
            .set_receive_preimage(&settled.id, &preimage(1))
            .await
            .unwrap();
        store
            .set_receive_invoice_status(&settled.id, HoldInvoiceStatus::Settled)
            .await
            .unwrap();
        hand_over(store, &returned).await;
        store.clear_receive_handover(&returned.id).await.unwrap();

        let after = read_receive(store, &other.id).await;
        assert_eq!(fields(&after), fields(&before));
    }

    pub(crate) async fn a_write_to_a_receive_the_store_does_not_have_stores_nothing(
        store: &dyn LightningStore,
    ) {
        stored_receive(store, 1).await;

        store
            .set_receive_handover("receive-2", &TransferId::generate(), 2_000)
            .await
            .unwrap();
        store.clear_receive_handover("receive-2").await.unwrap();
        store
            .set_receive_preimage("receive-2", &preimage(2))
            .await
            .unwrap();
        store
            .set_receive_invoice_status("receive-2", HoldInvoiceStatus::Settled)
            .await
            .unwrap();

        assert!(store.get_receive("receive-2").await.unwrap().is_none());
        let pending = store.pending_receives().await.unwrap();
        assert_eq!(receive_ids(&pending), ["receive-1"]);
    }

    pub(crate) async fn each_write_to_a_receive_moves_its_update_time_forward(
        store: &dyn LightningStore,
    ) {
        let (settled, returned) = (
            stored_receive(store, 1).await,
            stored_receive(store, 2).await,
        );
        let updated_at = async |receive: &LightningReceiveRecord| {
            read_receive(store, &receive.id).await.updated_at
        };
        let mut settled_times = vec![updated_at(&settled).await];

        a_moment().await;
        hand_over(store, &settled).await;
        hand_over(store, &returned).await;
        settled_times.push(updated_at(&settled).await);
        let returned_handed_over_at = updated_at(&returned).await;
        a_moment().await;
        store
            .set_receive_preimage(&settled.id, &preimage(1))
            .await
            .unwrap();
        settled_times.push(updated_at(&settled).await);
        a_moment().await;
        store
            .set_receive_invoice_status(&settled.id, HoldInvoiceStatus::Settled)
            .await
            .unwrap();
        settled_times.push(updated_at(&settled).await);
        store.clear_receive_handover(&returned.id).await.unwrap();

        assert!(settled_times.is_sorted_by(|earlier, later| earlier < later));
        assert!(returned_handed_over_at < updated_at(&returned).await);
    }

    pub(crate) async fn an_amount_too_large_for_the_database_is_refused(
        store: &dyn LightningStore,
    ) {
        let send_of_the_amount = LightningSendRecord {
            amount_sats: u64::MAX,
            ..new_send(1)
        };
        let send_with_the_fee = LightningSendRecord {
            fee_sats: u64::MAX,
            ..new_send(2)
        };
        let receive_of_the_amount = LightningReceiveRecord {
            amount_sats: u64::MAX,
            ..new_receive(1)
        };
        let receive = stored_receive(store, 2).await;

        assert!(store.insert_send(&send_of_the_amount).await.is_err());
        assert!(store.insert_send(&send_with_the_fee).await.is_err());
        assert!(store.insert_receive(&receive_of_the_amount).await.is_err());
        let handover = store
            .set_receive_handover(&receive.id, &handover_transfer(&receive), u64::MAX)
            .await;
        assert!(handover.is_err());

        assert!(store.get_send("send-1").await.unwrap().is_none());
        assert!(store.get_send("send-2").await.unwrap().is_none());
        assert!(store.get_receive("receive-1").await.unwrap().is_none());
        let stored = read_receive(store, &receive.id).await;
        assert_eq!(stored.transfer_id, None);
    }
}
