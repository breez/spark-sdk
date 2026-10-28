use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use bitcoin::secp256k1::{PublicKey, ecdsa, schnorr};
use spark::signer::{
    FrostJob, FrostShareResult, PrepareClaimRequest, PrepareLightningReceiveRequest,
    PrepareStaticDepositClaimRequest, PrepareStaticDepositRequest, PrepareTokenTransactionRequest,
    PrepareTransferRequest, PreparedClaim, PreparedLightningReceive, PreparedStaticDeposit,
    PreparedStaticDepositClaim, PreparedTokenTransaction, PreparedTransfer, SignQuoteRequest,
    SignSparkInvoiceRequest, SignStaticDepositRefundRequest, SignWatchtowerExitRecoveryRequest,
    SignedQuote, SignedSparkInvoice, SignerError, SparkSigner, StartStaticDepositRefundRequest,
    StartWatchtowerExitRecoveryRequest, StartedStaticDepositRefund, StartedWatchtowerExitRecovery,
};
use spark::tree::TreeNodeId;
use spark_wallet::{
    DefaultSigner, Network, Session, SessionStore, SessionStoreError, SparkSignerAdapter,
    SparkWallet, SparkWalletConfig, WalletBuilder,
};

pub struct CountingSparkSigner {
    inner: Arc<dyn SparkSigner>,
    auth_signatures: AtomicUsize,
}

impl CountingSparkSigner {
    pub fn new(inner: Arc<dyn SparkSigner>) -> Self {
        Self {
            inner,
            auth_signatures: AtomicUsize::new(0),
        }
    }

    pub fn auth_signatures(&self) -> usize {
        self.auth_signatures.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl SparkSigner for CountingSparkSigner {
    async fn get_identity_public_key(&self) -> Result<PublicKey, SignerError> {
        self.inner.get_identity_public_key().await
    }

    async fn get_public_key_for_leaf(
        &self,
        leaf_id: &TreeNodeId,
    ) -> Result<PublicKey, SignerError> {
        self.inner.get_public_key_for_leaf(leaf_id).await
    }

    async fn get_static_deposit_public_key(&self, index: u32) -> Result<PublicKey, SignerError> {
        self.inner.get_static_deposit_public_key(index).await
    }

    async fn sign_authentication_challenge(
        &self,
        challenge: &[u8],
    ) -> Result<ecdsa::Signature, SignerError> {
        self.auth_signatures.fetch_add(1, Ordering::SeqCst);
        self.inner.sign_authentication_challenge(challenge).await
    }

    async fn sign_message(&self, message: &[u8]) -> Result<ecdsa::Signature, SignerError> {
        self.inner.sign_message(message).await
    }

    async fn sign_leaf_refund_spend(
        &self,
        leaf_id: &TreeNodeId,
        sighash: &[u8],
    ) -> Result<schnorr::Signature, SignerError> {
        self.inner.sign_leaf_refund_spend(leaf_id, sighash).await
    }

    async fn sign_frost(&self, jobs: Vec<FrostJob>) -> Result<Vec<FrostShareResult>, SignerError> {
        self.inner.sign_frost(jobs).await
    }

    async fn prepare_transfer(
        &self,
        request: PrepareTransferRequest,
    ) -> Result<PreparedTransfer, SignerError> {
        self.inner.prepare_transfer(request).await
    }

    async fn prepare_claim(
        &self,
        request: PrepareClaimRequest,
    ) -> Result<PreparedClaim, SignerError> {
        self.inner.prepare_claim(request).await
    }

    async fn prepare_lightning_receive(
        &self,
        request: PrepareLightningReceiveRequest,
    ) -> Result<PreparedLightningReceive, SignerError> {
        self.inner.prepare_lightning_receive(request).await
    }

    async fn prepare_static_deposit(
        &self,
        request: PrepareStaticDepositRequest,
    ) -> Result<PreparedStaticDeposit, SignerError> {
        self.inner.prepare_static_deposit(request).await
    }

    async fn start_static_deposit_refund(
        &self,
        request: StartStaticDepositRefundRequest,
    ) -> Result<StartedStaticDepositRefund, SignerError> {
        self.inner.start_static_deposit_refund(request).await
    }

    async fn sign_static_deposit_refund(
        &self,
        request: SignStaticDepositRefundRequest,
    ) -> Result<frost_secp256k1_tr::Signature, SignerError> {
        self.inner.sign_static_deposit_refund(request).await
    }

    async fn start_watchtower_exit_recovery(
        &self,
        request: StartWatchtowerExitRecoveryRequest,
    ) -> Result<StartedWatchtowerExitRecovery, SignerError> {
        self.inner.start_watchtower_exit_recovery(request).await
    }

    async fn sign_watchtower_exit_recovery(
        &self,
        request: SignWatchtowerExitRecoveryRequest,
    ) -> Result<frost_secp256k1_tr::Signature, SignerError> {
        self.inner.sign_watchtower_exit_recovery(request).await
    }

    async fn prepare_static_deposit_claim(
        &self,
        request: PrepareStaticDepositClaimRequest,
    ) -> Result<PreparedStaticDepositClaim, SignerError> {
        self.inner.prepare_static_deposit_claim(request).await
    }

    async fn sign_spark_invoice(
        &self,
        request: SignSparkInvoiceRequest,
    ) -> Result<SignedSparkInvoice, SignerError> {
        self.inner.sign_spark_invoice(request).await
    }

    async fn prepare_token_transaction(
        &self,
        request: PrepareTokenTransactionRequest,
    ) -> Result<PreparedTokenTransaction, SignerError> {
        self.inner.prepare_token_transaction(request).await
    }

    async fn sign_quote(&self, request: SignQuoteRequest) -> Result<SignedQuote, SignerError> {
        self.inner.sign_quote(request).await
    }

    fn supports_split_transfers(&self) -> bool {
        self.inner.supports_split_transfers()
    }
}

#[derive(Default)]
pub struct ControlledSessionStore {
    sessions: Mutex<HashMap<PublicKey, Session>>,
}

impl ControlledSessionStore {
    pub fn snapshot(&self) -> HashMap<PublicKey, Session> {
        self.sessions
            .lock()
            .expect("sessions mutex poisoned")
            .clone()
    }

    pub fn restore(&self, sessions: HashMap<PublicKey, Session>) {
        *self.sessions.lock().expect("sessions mutex poisoned") = sessions;
    }

    pub fn update_all(&self, update: impl Fn(&mut Session)) {
        self.sessions
            .lock()
            .expect("sessions mutex poisoned")
            .values_mut()
            .for_each(update);
    }
}

#[async_trait::async_trait]
impl SessionStore for ControlledSessionStore {
    async fn get_session(
        &self,
        service_identity_key: &PublicKey,
    ) -> Result<Session, SessionStoreError> {
        self.sessions
            .lock()
            .expect("sessions mutex poisoned")
            .get(service_identity_key)
            .cloned()
            .ok_or(SessionStoreError::NotFound)
    }

    async fn set_session(
        &self,
        service_identity_key: &PublicKey,
        session: Session,
    ) -> Result<(), SessionStoreError> {
        self.sessions
            .lock()
            .expect("sessions mutex poisoned")
            .insert(*service_identity_key, session);
        Ok(())
    }
}

pub struct RefreshWallet {
    pub wallet: SparkWallet,
    pub signer: Arc<CountingSparkSigner>,
}

pub async fn build_refresh_wallet(
    config: SparkWalletConfig,
    seed: [u8; 32],
    store: Arc<ControlledSessionStore>,
) -> Result<RefreshWallet> {
    let signer = Arc::new(DefaultSigner::new(&seed, Network::Regtest)?);
    let counting = Arc::new(CountingSparkSigner::new(Arc::new(SparkSignerAdapter::new(
        signer,
    ))));
    let wallet = WalletBuilder::new(config, counting.clone())
        .with_session_store(store)
        .build()
        .await?;
    Ok(RefreshWallet {
        wallet,
        signer: counting,
    })
}
