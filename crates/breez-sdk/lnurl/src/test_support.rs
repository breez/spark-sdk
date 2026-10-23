//! Shared fixtures for the postgres-backed tests.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;

use axum::http::{Method, header::HOST};
use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::{Message, PublicKey, Secp256k1, SecretKey, ecdsa::Signature};
use spark::operator::rpc::{ConnectionManager, DefaultConnectionManager};
use spark::session_store::InMemorySessionStore;
use spark::ssp::ServiceProvider;
use spark::token::InMemoryTokenOutputStore;
use spark::tree::InMemoryTreeStore;
use spark_postgres::deadpool_postgres::Pool;
use spark_postgres::{PostgresStorageConfig, create_pool};
use spark_wallet::{
    DefaultSigner, Network, SparkSigner, SparkSignerAdapter, SparkWallet, SparkWalletConfig,
};
use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::sync::{Mutex, RwLock, watch};

use crate::postgresql::LnurlRepository;
use crate::state::State;

/// A pool over a throwaway postgres, migrated and empty of rows. The container
/// is returned with it and stops as soon as it is dropped, so a test has to
/// hold on to it for as long as it uses the pool.
pub async fn test_pool() -> (ContainerAsync<Postgres>, Pool) {
    let (container, pool) = empty_test_pool().await;
    crate::postgresql::run_migrations(&pool)
        .await
        .expect("run migrations");
    (container, pool)
}

/// A pool over a throwaway postgres with no schema applied.
pub async fn empty_test_pool() -> (ContainerAsync<Postgres>, Pool) {
    let container = Postgres::default()
        .start()
        .await
        .expect("start postgres container");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("get container port");

    let pool = create_pool(&PostgresStorageConfig::with_defaults(format!(
        "host=127.0.0.1 port={port} user=postgres password=postgres dbname=postgres"
    )))
    .expect("create test pool");

    (container, pool)
}

/// A repository over a pool from [`test_pool`].
pub async fn test_db() -> (ContainerAsync<Postgres>, crate::postgresql::LnurlRepository) {
    let (container, pool) = test_pool().await;
    (container, crate::postgresql::LnurlRepository::new(pool))
}

/// The state of a regtest server over `db`. Building it connects to nothing,
/// so every route works offline except the ones that create an invoice.
pub async fn test_state(db: LnurlRepository) -> State<LnurlRepository> {
    let network = Network::Regtest;
    let spark_config = SparkWalletConfig::default_config(network);
    let ssp_http_client = platform_utils::create_http_client(None).expect("create http client");
    let signer = Arc::new(DefaultSigner::new(&[1; 32], network).expect("create signer"));
    let spark_signer: Arc<dyn SparkSigner> = Arc::new(SparkSignerAdapter::new(signer.clone()));
    let session_store = Arc::new(InMemorySessionStore::default());
    let connection_manager: Arc<dyn ConnectionManager> = Arc::new(DefaultConnectionManager::new());
    let service_provider = Arc::new(ServiceProvider::new_with_client(
        spark_config.service_provider_config.clone(),
        spark_signer.clone(),
        session_store.clone(),
        None,
        Arc::clone(&ssp_http_client),
    ));
    let wallet = SparkWallet::new(
        spark_config.clone(),
        spark_signer,
        session_store.clone(),
        Arc::new(InMemoryTreeStore::default()),
        Arc::new(InMemoryTokenOutputStore::default()),
        Arc::clone(&connection_manager),
        Some(Arc::clone(&ssp_http_client)),
        None,
        None,
        None,
        None,
    )
    .await
    .expect("create wallet");
    let (invoice_paid_trigger, _) = watch::channel(());

    State {
        webhook_service: crate::webhooks::WebhookService::new(db.clone()),
        db,
        wallet: Arc::new(wallet),
        is_mainnet: false,
        scheme: "https".to_string(),
        min_sendable: 1000,
        max_sendable: 4_000_000_000,
        include_spark_address: false,
        registration_limit: None,
        domains: Arc::new(RwLock::new(HashMap::new())),
        rewrites: Arc::new(RwLock::new(HashMap::new())),
        nostr_keys: None,
        ca_cert: None,
        crl_url: None,
        crl: HashSet::new(),
        connection_manager,
        coordinator: spark_config.operator_pool.get_coordinator().clone(),
        signer,
        session_store,
        service_provider,
        spark_config,
        ssp_http_client,
        jwt_cache: None,
        subscribed_keys: Arc::new(Mutex::new(HashSet::new())),
        invoice_paid_trigger,
        webhook_secret: "secret".to_string(),
    }
}

/// A router listening on a local port.
pub struct TestServer {
    address: SocketAddr,
    client: reqwest::Client,
}

impl TestServer {
    pub async fn start(router: axum::Router) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a local port");
        let address = listener.local_addr().expect("read the local address");
        tokio::spawn(async move { axum::serve(listener, router).await });
        Self {
            address,
            client: reqwest::Client::new(),
        }
    }

    /// A request for `domain`, which is what a reverse proxy in front of the
    /// server passes on in `Host`.
    pub fn request(&self, method: Method, domain: &str, path: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, format!("http://{}{path}", self.address))
            .header(HOST, domain)
    }
}

/// Deterministic keypair from a seed byte.
pub fn transfer_key(seed: u8) -> (SecretKey, PublicKey) {
    let secp = Secp256k1::new();
    let secret = SecretKey::from_slice(&[seed; 32]).expect("valid secret key");
    let public = PublicKey::from_secret_key(&secp, &secret);
    (secret, public)
}

/// Sign `message` the way the SDK does: ECDSA over `sha256(message)`.
pub fn sign(secret: &SecretKey, message: &str) -> Signature {
    let secp = Secp256k1::new();
    let digest = sha256::Hash::hash(message.as_bytes());
    secp.sign_ecdsa(&Message::from_digest(digest.to_byte_array()), secret)
}
