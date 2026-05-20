use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use breez_sdk_spark::*;
use log::info;

pub(crate) async fn init_sdk_advanced() -> Result<BreezSdk> {
    // ANCHOR: init-sdk-advanced
    // Construct the seed using a mnemonic, entropy or passkey
    let mnemonic = "<mnemonic words>".to_string();
    let seed = Seed::Mnemonic {
        mnemonic,
        passphrase: None,
    };

    // Create the default config
    let mut config = default_config(Network::Mainnet);
    config.api_key = Some("<breez api key>".to_string());

    // Build the SDK using the config, seed and default storage
    let builder = SdkBuilder::new(config, seed).with_default_storage("./.data".to_string());
    // You can also pass your custom implementations:
    // let builder = builder.with_storage(<your storage implementation>)
    // let builder = builder.with_chain_service(<your chain service implementation>)
    // let builder = builder.with_rest_client(<your rest client implementation>)
    // let builder = builder.with_key_set(KeySetConfig { key_set_type: <your key set type>, use_address_index: <use address index>, account_number: <account number> })
    // let builder = builder.with_payment_observer(<your payment observer implementation>);
    let sdk = builder.build().await?;

    // ANCHOR_END: init-sdk-advanced
    Ok(sdk)
}

pub(crate) fn with_rest_chain_service(builder: SdkBuilder) -> SdkBuilder {
    // ANCHOR: with-rest-chain-service
    let url = "<your REST chain service URL>".to_string();
    let chain_api_type = ChainApiType::MempoolSpace;
    let optional_credentials = Credentials {
        username: "<username>".to_string(),
        password: "<password>".to_string(),
    };
    builder.with_rest_chain_service(url, chain_api_type, Some(optional_credentials))
    // ANCHOR_END: with-rest-chain-service
}

pub(crate) fn with_key_set(builder: SdkBuilder) -> SdkBuilder {
    // ANCHOR: with-key-set
    let key_set_type = KeySetType::Default;
    let use_address_index = false;
    let optional_account_number = 21;
    builder.with_key_set(KeySetConfig {
        key_set_type,
        use_address_index,
        account_number: Some(optional_account_number),
    })
    // ANCHOR_END: with-key-set
}

// ANCHOR: with-payment-observer
pub(crate) struct ExamplePaymentObserver {}

#[async_trait]
impl PaymentObserver for ExamplePaymentObserver {
    async fn before_send(
        &self,
        payments: Vec<ProvisionalPayment>,
    ) -> Result<(), PaymentObserverError> {
        for payment in payments {
            info!(
                "About to send payment: {:?} of amount {:?}",
                payment.payment_id, payment.amount
            );
        }
        Ok(())
    }
}

pub(crate) fn with_payment_observer(builder: SdkBuilder) -> SdkBuilder {
    let observer = ExamplePaymentObserver {};
    builder.with_payment_observer(Arc::new(observer))
}
// ANCHOR_END: with-payment-observer

pub(crate) async fn init_sdk_postgres() -> Result<BreezSdk> {
    // ANCHOR: init-sdk-postgres
    // Construct the seed using a mnemonic, entropy or passkey
    let mnemonic = "<mnemonic words>".to_string();
    let seed = Seed::Mnemonic {
        mnemonic,
        passphrase: None,
    };

    // Create the default config
    let mut config = default_config(Network::Mainnet);
    config.api_key = Some("<breez api key>".to_string());

    // Configure PostgreSQL backend
    // Connection string format: "host=localhost user=postgres password=secret dbname=spark"
    // Or URI format: "postgres://user:password@host:port/dbname?sslmode=require"
    let mut postgres_config =
        default_postgres_storage_config("host=localhost user=postgres dbname=spark".to_string());
    // Optionally pool settings can be adjusted. Some examples:
    postgres_config.max_pool_size = 8; // Max connections in pool
    postgres_config.wait_timeout_secs = Some(30); // Timeout waiting for connection
    // If your service owns SDK-compatible schema migrations:
    postgres_config.run_migration = false;

    // Build the SDK with the PostgreSQL storage backend (storage, tree store,
    // and token store). Per-tenant scoping (rows isolated by seed identity) is
    // applied automatically.
    let sdk = SdkBuilder::new(config, seed)
        .with_storage_backend(postgres_storage(postgres_config))
        .build()
        .await?;
    // ANCHOR_END: init-sdk-postgres

    Ok(sdk)
}

pub(crate) async fn init_sdk_mysql() -> Result<BreezSdk> {
    // ANCHOR: init-sdk-mysql
    // Construct the seed using a mnemonic, entropy or passkey
    let mnemonic = "<mnemonic words>".to_string();
    let seed = Seed::Mnemonic {
        mnemonic,
        passphrase: None,
    };

    // Create the default config
    let mut config = default_config(Network::Mainnet);
    config.api_key = Some("<breez api key>".to_string());

    // Configure MySQL backend (MySQL 8.0+).
    // Connection string format (URL only):
    //   "mysql://user:password@host:3306/dbname?ssl-mode=required"
    let mut mysql_config = default_mysql_storage_config(
        "mysql://user:password@localhost:3306/spark".to_string(),
    );
    // Optionally pool settings can be adjusted. Some examples:
    mysql_config.max_pool_size = 8; // Max connections in pool
    mysql_config.recycle_timeout_secs = Some(60); // Recycle idle connections after this many seconds
    // Provide a custom CA certificate when using ssl-mode=verify_ca or verify_identity:
    // mysql_config.root_ca_pem = Some("-----BEGIN CERTIFICATE-----\n...".to_string());

    // Build the SDK with the MySQL storage backend (storage, tree store, and
    // token store). Per-tenant scoping (rows isolated by seed identity) is
    // applied automatically.
    let sdk = SdkBuilder::new(config, seed)
        .with_storage_backend(mysql_storage(mysql_config))
        .build()
        .await?;
    // ANCHOR_END: init-sdk-mysql

    Ok(sdk)
}

pub(crate) async fn init_sdk_server() -> Result<BreezSdk> {
    // ANCHOR: init-sdk-server
    // Construct the seed using a mnemonic, entropy or passkey
    let mnemonic = "<mnemonic words>".to_string();
    let seed = Seed::Mnemonic {
        mnemonic,
        passphrase: None,
    };

    // Build a server-mode config: same as default_config(network) with
    // background_tasks_enabled = false. No periodic sync, no real-time sync
    // client, no leaf/token optimizer, no flashnet refunder, no lightning-
    // address recovery, no spark private-mode init.
    let mut config = default_server_config(Network::Mainnet);
    config.api_key = Some("<breez api key>".to_string());

    // Typically server-mode SDKs are built per request and share infrastructure
    // (DB pool, REST chain service, SSP/Connection Manager) across instances.
    // Pass the shared resources via the builder; see the "Customizing the SDK"
    // page for each component.
    let sdk = SdkBuilder::new(config, seed)
        .with_default_storage("./.data".to_string())
        .build()
        .await?;
    // ANCHOR_END: init-sdk-server

    Ok(sdk)
}

pub(crate) async fn server_mode_request_handler(sdk: &BreezSdk) -> Result<String> {
    // ANCHOR: server-mode-request-handler
    // User-facing request handler: do not call sync_wallet here. Operations
    // that read from local storage (get_info, list_payments, etc.) do not
    // need a defensive sync. Call sync_wallet only from webhook handlers or
    // reconciliation jobs that need to observe an external state change.
    let response = sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::Bolt11Invoice {
                description: "<invoice description>".to_string(),
                amount_sats: Some(5_000),
                expiry_secs: Some(3600),
                payment_hash: None,
            },
        })
        .await?;

    // Always disconnect at the end of the request lifecycle to flush
    // outstanding storage writes. See [Disconnecting](initializing.md).
    sdk.disconnect().await?;
    // ANCHOR_END: server-mode-request-handler
    Ok(response.payment_request)
}

pub(crate) async fn server_mode_provisioning(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: server-mode-provisioning
    // One-time setup when a wallet is first registered. The client-mode SDK
    // would normally apply the private-mode preset itself on first startup;
    // server-mode SDKs do not, so opt in once here via update_user_settings.
    sdk.update_user_settings(UpdateUserSettingsRequest {
        spark_private_mode_enabled: Some(true),
        stable_balance_active_label: None,
    })
    .await?;

    sdk.disconnect().await?;
    // ANCHOR_END: server-mode-provisioning
    Ok(())
}

pub(crate) async fn refund_pending_conversions(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: refund-pending-conversions
    // The flashnet conversion refunder doesn't run in the background in server
    // mode. Call this from your own scheduler (e.g. once per minute) to issue
    // pending refunds for failed conversions.
    sdk.refund_pending_conversions().await?;
    // ANCHOR_END: refund-pending-conversions
    Ok(())
}
