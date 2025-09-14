mod flashnet;
mod utils;
use bitcoin::secp256k1::PublicKey;
use spark_wallet::ConnectionManager;
use spark_wallet::DefaultConnectionManager;
use spark_wallet::Identifier;
use spark_wallet::OperatorConfig;
use spark_wallet::Signer;
use spark_wallet::SparkWalletError;
use spark_wallet::WalletBuilder;
use spark_wallet::{
    Network, OperatorPoolConfig, ServiceProviderConfig, SparkWallet, SparkWalletConfig,
    TokensConfig,
};
use std::str::FromStr;
use std::sync::Arc;

use once_cell::sync::Lazy;

use crate::flashnet::SessionStore;
use crate::flashnet::SignerStore;
use crate::flashnet::UserSessionManager;

const NETWORK: Network = Network::Regtest;

static SPARK_CONFIG: Lazy<SparkWalletConfig> = Lazy::new(|| SparkWalletConfig {
    network: NETWORK,
    operator_pool: OperatorPoolConfig::new(
        0,
        vec![
            OperatorConfig {
                id: 0,
                identifier: Identifier::deserialize(
                    &hex::decode(
                        "0000000000000000000000000000000000000000000000000000000000000001",
                    )
                    .unwrap(),
                )
                .unwrap(),
                address: "https://0.spark.lightspark.com".to_string(),
                identity_public_key: PublicKey::from_str(
                    "03dfbdff4b6332c220f8fa2ba8ed496c698ceada563fa01b67d9983bfc5c95e763",
                )
                .unwrap(),
            },
            OperatorConfig {
                id: 1,
                identifier: Identifier::deserialize(
                    &hex::decode(
                        "0000000000000000000000000000000000000000000000000000000000000002",
                    )
                    .unwrap(),
                )
                .unwrap(),
                address: "https://1.spark.lightspark.com".to_string(),
                identity_public_key: PublicKey::from_str(
                    "03e625e9768651c9be268e287245cc33f96a68ce9141b0b4769205db027ee8ed77",
                )
                .unwrap(),
            },
            OperatorConfig {
                id: 2,
                identifier: Identifier::deserialize(
                    &hex::decode(
                        "0000000000000000000000000000000000000000000000000000000000000003",
                    )
                    .unwrap(),
                )
                .unwrap(),
                address: "https://2.spark.flashnet.xyz".to_string(),
                identity_public_key: PublicKey::from_str(
                    "022eda13465a59205413086130a65dc0ed1b8f8e51937043161f8be0c369b1a410",
                )
                .unwrap(),
            },
        ],
    )
    .unwrap(),
    reconnect_interval_seconds: 1,
    service_provider_config: ServiceProviderConfig {
        base_url: "https://api.lightspark.com".to_string(),
        identity_public_key: PublicKey::from_str(
            "022bf283544b16c0622daecb79422007d167eca6ce9f0c98c0c49833b1f7170bfe",
        )
        .unwrap(),
        schema_endpoint: Some("graphql/spark/2025-03-19".to_string()),
    },
    split_secret_threshold: 2,
    tokens_config: TokensConfig {
        expected_withdraw_bond_sats: 10_000,
        expected_withdraw_relative_block_locktime: 1_000,
        transaction_validity_duration_seconds: 180,
    },
});

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup
    let (store, sender_public_key, receiver_public_key) = utils::init_store().await;

    // wallet factory to create cheap wallets per request with reused resources
    let wallet_factory = WalletFactory::new(store.clone());

    // Cosntruct the wallet
    let sender_wallet = wallet_factory.create_wallet(&sender_public_key).await?;
    let receiver_wallet = wallet_factory.create_wallet(&receiver_public_key).await?;

    // Sync both wallets
    sender_wallet.sync().await?;
    receiver_wallet.sync().await?;

    // Log balances
    let sender_balance = sender_wallet.get_balance().await?;
    println!("Sender balance: {sender_balance} sats");
    let receiver_balance = receiver_wallet.get_balance().await?;
    println!("Receiver balance: {receiver_balance} sats");

    // Sender sends 1000 sats to the receiver
    let transfer = sender_wallet
        .transfer(1000, &utils::spark_address(receiver_public_key))
        .await?;
    println!("Sent transfer ID: {}", transfer.id);

    // Receiver claims the transfer
    let wallet_transfer = receiver_wallet.claim_pending_transfers().await?;
    println!("Claimed {} transfers", wallet_transfer.len());

    Ok(())
}

struct WalletFactory {
    config: SparkWalletConfig,
    store: Arc<dyn SignerStore>,
    session_store: Arc<SessionStore>,
    connection_manager: Arc<dyn ConnectionManager>,
}

impl WalletFactory {
    pub fn new(store: Arc<dyn SignerStore>) -> Self {
        Self {
            store,
            config: SPARK_CONFIG.clone(),
            session_store: Arc::new(SessionStore::default()),
            connection_manager: Arc::new(DefaultConnectionManager::default()),
        }
    }

    // cheap wallet creation that reuses sessions and connections accross different wallets/users
    pub async fn create_wallet(
        &self,
        public_key: &PublicKey,
    ) -> Result<SparkWallet, SparkWalletError> {
        let signer: Arc<dyn Signer> =
            Arc::new(utils::user_signer(self.store.clone(), public_key).unwrap());
        WalletBuilder::new(self.config.clone(), signer)
            .with_connection_manager(self.connection_manager.clone())
            .with_session_manager(Arc::new(UserSessionManager {
                user_public_key: *public_key,
                session_store: self.session_store.clone(),
            }))
            .with_background_processing(false) // no background processing, use wants controll over the claim process
            .build()
            .await
    }
}
