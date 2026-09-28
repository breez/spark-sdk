use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use bitcoin::hashes::{Hash as _, sha256};
use hex::ToHex;
use spark::operator::OperatorPool;
use spark::operator::rpc::{ConnectionManager, DefaultConnectionManager};
use spark::services::{HtlcService, Swap, TimelockManager, Transfer, TransferService};
use spark::session_store::InMemorySessionStore;
use spark::signer::{Signer, SparkSigner, SparkSignerAdapter};
use spark::ssp::{RequestLightningReceiveInput, RequestLightningSendInput, ServiceProvider};
use spark::tree::{InMemoryTreeStore, SynchronousTreeService, TreeNodeId, TreeService, TreeStore};
use spark::utils::leaf_key_tweak::with_node_id_keys;
use spark_itest::helpers::deposit_with_amount;
use spark_itest::lightning_stack::LightningStack;
use spark_wallet::{ServiceProviderConfig, SparkWalletConfig};
use sspd_lib::lightning::ldk::proto::{api, types};
use sspd_lib::lightning::node::LightningNode;
use tracing::info;

async fn counterparty_invoice(stack: &LightningStack, amount_sats: u64) -> Result<String> {
    let resp: api::Bolt11ReceiveResponse = stack
        .counterparty
        .call_for_test(
            "Bolt11Receive",
            &api::Bolt11ReceiveRequest {
                amount_msat: Some(amount_sats * 1000),
                description: Some(types::Bolt11InvoiceDescription {
                    kind: Some(types::bolt11_invoice_description::Kind::Direct(
                        "adversarial".to_string(),
                    )),
                }),
                expiry_secs: 3600,
            },
        )
        .await?;
    Ok(resp.invoice)
}

async fn payment_hash_of(stack: &LightningStack, invoice: &str) -> Result<sha256::Hash> {
    let decoded = stack.counterparty.decode_invoice(invoice).await?;
    Ok(sha256::Hash::from_byte_array(decoded.payment_hash))
}

async fn client_create_htlc(
    config: &SparkWalletConfig,
    signer: Arc<dyn Signer>,
    ssp_config: &ServiceProviderConfig,
    receiver_pubkey: &bitcoin::secp256k1::PublicKey,
    payment_hash: &sha256::Hash,
    leaf_id: &TreeNodeId,
) -> Result<Transfer> {
    let network = config.network;
    let spark_signer: Arc<dyn SparkSigner> = Arc::new(SparkSignerAdapter::new(signer.clone()));
    let identity_public_key = spark::signer::derive_identity_public_key(signer.as_ref()).await?;
    let session_store = Arc::new(InMemorySessionStore::default());
    let connection_manager: Arc<dyn ConnectionManager> = Arc::new(DefaultConnectionManager::new());
    let operator_pool = Arc::new(
        OperatorPool::connect(
            &config.operator_pool,
            connection_manager,
            session_store.clone(),
            spark_signer.clone(),
            None,
        )
        .await?,
    );
    let transfer_service = Arc::new(TransferService::new(
        spark_signer.clone(),
        network,
        config.split_secret_threshold,
        operator_pool.clone(),
        None,
    ));
    let service_provider = Arc::new(ServiceProvider::new(
        ssp_config.clone(),
        spark_signer.clone(),
        session_store,
        None,
    )?);
    let swap_service = Arc::new(Swap::new(
        network,
        operator_pool.clone(),
        spark_signer.clone(),
        service_provider,
        transfer_service.clone(),
    ));
    let timelock_manager = Arc::new(TimelockManager::new(
        spark_signer.clone(),
        network,
        operator_pool.clone(),
    ));
    let tree_store: Arc<dyn TreeStore> = Arc::new(InMemoryTreeStore::default());
    let tree_service = SynchronousTreeService::new(
        identity_public_key,
        network,
        operator_pool.clone(),
        tree_store,
        timelock_manager,
        spark_signer.clone(),
        Some(swap_service),
        None,
    );
    let htlc_service = HtlcService::new(
        operator_pool.clone(),
        network,
        spark_signer.clone(),
        transfer_service,
        None,
    );
    tree_service.refresh_leaves().await?;
    let leaves = tree_service.list_leaves().await?;
    let node = leaves
        .available
        .iter()
        .find(|n| n.id == *leaf_id)
        .ok_or_else(|| anyhow::anyhow!("leaf not available: {leaf_id}"))?
        .clone();
    // A wallet's Lightning send expiry: the daemon caps the route to resolve
    // before the transfer expires.
    let expiry_time = SystemTime::now() + Duration::from_secs(16 * 24 * 60 * 60);
    Ok(htlc_service
        .create_htlc(
            &with_node_id_keys(vec![node]),
            receiver_pubkey,
            payment_hash,
            expiry_time,
            None,
        )
        .await?)
}

#[tokio::test]
#[test_log::test]
async fn send_rejects_payment_hash_mismatch() -> Result<()> {
    let stack = LightningStack::start().await?;
    deposit_with_amount(&stack.alice, &stack.fixtures.bitcoind, 16384).await?;
    let leaf = stack.alice.list_leaves().await?.available[0].id.clone();

    let committed_hash = sha256::Hash::hash(&[2u8; 32]);
    let transfer = client_create_htlc(
        &stack.alice_config,
        stack.alice_signer.clone(),
        &stack.ssp_config,
        &stack.sspd.identity_public_key,
        &committed_hash,
        &leaf,
    )
    .await?;
    let invoice = counterparty_invoice(&stack, 1_000).await?;
    assert_ne!(
        payment_hash_of(&stack, &invoice).await?,
        committed_hash,
        "the invoice has to be for a different hash for this to test anything"
    );

    let result = stack
        .ssp_client()?
        .request_lightning_send(RequestLightningSendInput {
            encoded_invoice: invoice,
            idempotency_key: None,
            amount_sats: Some(1_000),
            user_outbound_transfer_external_id: Some(transfer.id.to_string()),
        })
        .await;

    let error = result
        .err()
        .context("the daemon must refuse an invoice for a hash the user did not commit to")?;
    info!("got the expected refusal: {error:?}");
    Ok(())
}

#[tokio::test]
#[test_log::test]
async fn send_rejects_insufficient_committed_amount() -> Result<()> {
    let stack = LightningStack::start().await?;
    deposit_with_amount(&stack.alice, &stack.fixtures.bitcoind, 1024).await?;
    let leaf = stack.alice.list_leaves().await?.available[0].id.clone();

    let invoice = counterparty_invoice(&stack, 100_000).await?;
    let payment_hash = payment_hash_of(&stack, &invoice).await?;
    let transfer = client_create_htlc(
        &stack.alice_config,
        stack.alice_signer.clone(),
        &stack.ssp_config,
        &stack.sspd.identity_public_key,
        &payment_hash,
        &leaf,
    )
    .await?;

    let result = stack
        .ssp_client()?
        .request_lightning_send(RequestLightningSendInput {
            encoded_invoice: invoice,
            idempotency_key: None,
            amount_sats: Some(100_000),
            user_outbound_transfer_external_id: Some(transfer.id.to_string()),
        })
        .await;

    let error = result
        .err()
        .context("the daemon must refuse to front more than the user committed")?;
    info!("got the expected refusal: {error:?}");
    Ok(())
}

#[tokio::test]
#[test_log::test]
async fn receive_gives_no_leaves_until_htlc_held() -> Result<()> {
    let stack = LightningStack::start().await?;
    let alice_identity =
        spark::signer::derive_identity_public_key(stack.alice_signer.as_ref()).await?;
    let payment_hash = sha256::Hash::hash(&[7u8; 32]);

    let request = stack
        .ssp_client()?
        .request_lightning_receive(RequestLightningReceiveInput {
            receiver_identity_pubkey: Some(alice_identity.serialize().to_vec().encode_hex()),
            amount_sats: 16_384,
            network: stack.alice_config.network.into(),
            payment_hash: payment_hash.to_byte_array().encode_hex(),
            description_hash: None,
            expiry_secs: Some(3600),
            memo: Some("adversarial".to_string()),
            include_spark_address: false,
            spark_invoice: None,
        })
        .await?;
    info!("daemon issued the invoice for receive {}", request.id);

    // Long enough for the daemon's receive worker to run on its timer more than
    // once.
    tokio::time::sleep(Duration::from_secs(25)).await;

    let receive = stack
        .lightning_request(&request.id)
        .await?
        .receive
        .context("the daemon should still know the receive")?;
    assert!(
        receive.transfer_id.is_none(),
        "no leaves may be handed over before the incoming HTLC is held"
    );
    assert_eq!(
        receive.invoice_status, "pending",
        "the receive stays open while nothing has arrived"
    );
    Ok(())
}
