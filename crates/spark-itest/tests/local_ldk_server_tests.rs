use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use bitcoin::hashes::{Hash as _, sha256};
use bitcoin::secp256k1::PublicKey;
use bitcoin::{Address, Amount, Network};
use hex::ToHex;
use spark::operator::OperatorPool;
use spark::operator::rpc::{ConnectionManager, DefaultConnectionManager};
use spark::services::{HtlcService, Preimage, Swap, TimelockManager, Transfer, TransferService};
use spark::session_store::InMemorySessionStore;
use spark::signer::{Signer, SparkSigner, SparkSignerAdapter};
use spark::ssp::{RequestLightningReceiveInput, RequestLightningSendInput, ServiceProvider};
use spark::tree::{InMemoryTreeStore, SynchronousTreeService, TreeNodeId, TreeService, TreeStore};
use spark::utils::leaf_key_tweak::with_node_id_keys;
use spark_itest::fixtures::bitcoind::BitcoindFixture;
use spark_itest::fixtures::ldk_server::LdkServerFixture;
use spark_itest::helpers::deposit_with_amount;
use spark_itest::lightning_stack::LightningStack;
use spark_wallet::{ServiceProviderConfig, SparkWalletConfig};
use sspd_lib::lightning::ldk::LdkServerNode;
use sspd_lib::lightning::ldk::proto::{api, types};
use sspd_lib::lightning::node::LightningNode;
use sspd_lib::lightning::receive::min_blocks_until_deadline;
use sspd_lib::wakeup::Wakeup;
use tokio::time::sleep;
use tracing::info;

const CHANNEL_CONFIRMATIONS: u64 = 6;

const SETTLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Well inside the receive worker's 60 second backup timer, so a daemon that
/// misses the operators' event for an applied handover most likely fails it.
const EVENT_SETTLE_TIMEOUT: Duration = Duration::from_secs(10);

const POLL: Duration = Duration::from_millis(100);

async fn setup_channel(stack: &LightningStack) -> Result<()> {
    setup_channel_with_config(stack, None).await
}

async fn setup_channel_with_config(
    stack: &LightningStack,
    channel_config: Option<types::ChannelConfig>,
) -> Result<()> {
    open_channel_between(
        &stack.ssp_node,
        &stack.counterparty,
        &stack.counterparty_peer_address(),
        &stack.fixtures.bitcoind,
        channel_config,
    )
    .await
}

async fn open_channel_between(
    opener: &LdkServerNode,
    peer: &LdkServerNode,
    peer_address: &str,
    bitcoind: &BitcoindFixture,
    channel_config: Option<types::ChannelConfig>,
) -> Result<()> {
    let peer_info: api::GetNodeInfoResponse = peer
        .call_for_test("GetNodeInfo", &api::GetNodeInfoRequest {})
        .await?;

    let recv: api::OnchainReceiveResponse = opener
        .call_for_test("OnchainReceive", &api::OnchainReceiveRequest {})
        .await?;
    let address = Address::from_str(&recv.address)?.require_network(Network::Regtest)?;
    bitcoind
        .fund_address(&address, Amount::from_sat(1_000_000))
        .await?;
    bitcoind.generate_blocks(CHANNEL_CONFIRMATIONS).await?;

    wait_until("the opening node sees its funds", || async {
        let balances: api::GetBalancesResponse = opener
            .call_for_test("GetBalances", &api::GetBalancesRequest {})
            .await?;
        Ok(balances.spendable_onchain_balance_sats >= 300_000)
    })
    .await?;

    let _: api::ConnectPeerResponse = opener
        .call_for_test(
            "ConnectPeer",
            &api::ConnectPeerRequest {
                node_pubkey: peer_info.node_id.clone(),
                address: peer_address.to_string(),
                persist: true,
            },
        )
        .await?;
    let _: api::OpenChannelResponse = opener
        .call_for_test(
            "OpenChannel",
            &api::OpenChannelRequest {
                node_pubkey: peer_info.node_id,
                address: peer_address.to_string(),
                channel_amount_sats: 200_000,
                push_to_counterparty_msat: Some(100_000_000),
                channel_config,
                announce_channel: false,
                disable_counterparty_reserve: false,
            },
        )
        .await?;

    // Mines on every poll: the funding transaction is broadcast some time after
    // the open returns.
    wait_until("the channel is usable on both sides", || async {
        bitcoind.generate_blocks(1).await?;
        for node in [opener, peer] {
            let channels: api::ListChannelsResponse = node
                .call_for_test("ListChannels", &api::ListChannelsRequest {})
                .await?;
            if !channels.channels.iter().any(|c| c.is_usable) {
                return Ok(false);
            }
        }
        Ok(true)
    })
    .await?;
    info!("channel usable on both sides");
    Ok(())
}

async fn wait_until<F, Fut>(what: &str, mut condition: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<bool>>,
{
    let deadline = SystemTime::now() + SETTLE_TIMEOUT;
    loop {
        if condition().await? {
            return Ok(());
        }
        if SystemTime::now() >= deadline {
            anyhow::bail!("timed out waiting for {what}");
        }
        sleep(POLL).await;
    }
}

async fn wait_for_ok<T, F, Fut>(what: &str, mut attempt: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let deadline = SystemTime::now() + SETTLE_TIMEOUT;
    loop {
        match attempt().await {
            Ok(value) => return Ok(value),
            Err(e) if SystemTime::now() >= deadline => {
                return Err(e).with_context(|| format!("timed out waiting for {what}"));
            }
            Err(_) => sleep(POLL).await,
        }
    }
}

async fn client_create_htlc(
    config: &SparkWalletConfig,
    signer: Arc<dyn Signer>,
    ssp_config: &ServiceProviderConfig,
    receiver_pubkey: &PublicKey,
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

async fn client_claim_htlc(
    config: &SparkWalletConfig,
    signer: Arc<dyn Signer>,
    preimage: &Preimage,
) -> Result<u64> {
    let network = config.network;
    let spark_signer: Arc<dyn SparkSigner> = Arc::new(SparkSignerAdapter::new(signer.clone()));
    let session_store = Arc::new(InMemorySessionStore::default());
    let connection_manager: Arc<dyn ConnectionManager> = Arc::new(DefaultConnectionManager::new());
    let operator_pool = Arc::new(
        OperatorPool::connect(
            &config.operator_pool,
            connection_manager,
            session_store,
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
    let htlc_service = HtlcService::new(
        operator_pool.clone(),
        network,
        spark_signer.clone(),
        transfer_service.clone(),
        None,
    );
    let transfer = htlc_service.provide_preimage(preimage).await?;
    let claimed = transfer_service.claim_transfer(&transfer, None).await?;
    Ok(claimed.iter().map(|node| node.value).sum())
}

async fn client_store_preimage_shares(
    config: &SparkWalletConfig,
    signer: Arc<dyn Signer>,
    preimage: &Preimage,
    invoice_string: String,
) -> Result<()> {
    let session_store = Arc::new(InMemorySessionStore::default());
    let connection_manager: Arc<dyn ConnectionManager> = Arc::new(DefaultConnectionManager::new());
    let spark_signer: Arc<dyn SparkSigner> = Arc::new(SparkSignerAdapter::new(signer.clone()));
    let operator_pool = OperatorPool::connect(
        &config.operator_pool,
        connection_manager,
        session_store,
        spark_signer.clone(),
        None,
    )
    .await?;
    spark::services::store_preimage_shares(
        &operator_pool,
        &signer,
        config.split_secret_threshold,
        preimage,
        invoice_string,
        spark::signer::derive_identity_public_key(signer.as_ref()).await?,
    )
    .await?;
    Ok(())
}

/// The node records a payment when its invoice is issued, but writes the claim
/// deadline only once the HTLC is held.
async fn held_htlc_deadline(node: &LdkServerNode, payment_hash: &[u8; 32]) -> Result<Option<u32>> {
    use sspd_lib::lightning::ldk::proto::types::payment_kind::Kind;

    let mut page_token = None;
    loop {
        let resp: api::ListPaymentsResponse = node
            .call_for_test("ListPayments", &api::ListPaymentsRequest { page_token })
            .await?;
        for payment in &resp.payments {
            let Some(Kind::Bolt11(bolt11)) = payment.kind.as_ref().and_then(|k| k.kind.as_ref())
            else {
                continue;
            };
            if hex::decode(&bolt11.hash).ok().as_deref() == Some(payment_hash.as_slice())
                && let Some(deadline) = bolt11.claim_deadline
            {
                return Ok(Some(deadline));
            }
        }
        match resp.next_page_token {
            Some(next) => page_token = Some(next),
            None => return Ok(None),
        }
    }
}

async fn counterparty_invoice(stack: &LightningStack, amount_sats: u64) -> Result<String> {
    let resp: api::Bolt11ReceiveResponse = stack
        .counterparty
        .call_for_test(
            "Bolt11Receive",
            &api::Bolt11ReceiveRequest {
                amount_msat: Some(amount_sats * 1000),
                description: Some(types::Bolt11InvoiceDescription {
                    kind: Some(types::bolt11_invoice_description::Kind::Direct(
                        "itest".to_string(),
                    )),
                }),
                expiry_secs: 3600,
            },
        )
        .await?;
    Ok(resp.invoice)
}

async fn request_send(
    stack: &LightningStack,
    invoice: &str,
    amount_sats: u64,
    transfer: &Transfer,
) -> Result<String> {
    let request = stack
        .ssp_client()?
        .request_lightning_send(RequestLightningSendInput {
            encoded_invoice: invoice.to_string(),
            idempotency_key: None,
            amount_sats: Some(amount_sats),
            user_outbound_transfer_external_id: Some(transfer.id.to_string()),
        })
        .await?;
    Ok(request.id)
}

async fn send_record(
    stack: &LightningStack,
    id: &str,
) -> Result<spark_itest::fixtures::sspd::internal_api::LightningSend> {
    stack
        .lightning_request(id)
        .await?
        .send
        .context("the daemon has no send under that id")
}

#[tokio::test]
#[test_log::test]
async fn send_over_real_channel() -> Result<()> {
    let stack = LightningStack::start().await?;
    setup_channel(&stack).await?;

    deposit_with_amount(&stack.alice, &stack.fixtures.bitcoind, 16384).await?;
    let leaf = stack.alice.list_leaves().await?.available[0].id.clone();

    let invoice = counterparty_invoice(&stack, 10_000).await?;
    let decoded = stack.ssp_node.decode_invoice(&invoice).await?;
    let payment_hash = sha256::Hash::from_byte_array(decoded.payment_hash);
    let transfer = client_create_htlc(
        &stack.alice_config,
        stack.alice_signer.clone(),
        &stack.ssp_config,
        &stack.sspd.identity_public_key,
        &payment_hash,
        &leaf,
    )
    .await?;
    info!("Alice committed her leaf via HTLC transfer {}", transfer.id);

    let id = request_send(&stack, &invoice, 10_000, &transfer).await?;
    assert_eq!(send_record(&stack, &id).await?.amount_sats, 10_000);

    wait_until("the daemon to complete the send", || async {
        Ok(send_record(&stack, &id).await?.is_complete)
    })
    .await?;

    let record = send_record(&stack, &id).await?;
    assert!(record.has_preimage, "the daemon learned the real preimage");
    assert!(
        record.leaves_claimed,
        "the daemon took the leaf the user committed"
    );

    let counts = stack.sspd.pool_leaf_counts().await?;
    assert!(
        counts.contains_key(&16384),
        "Alice's leaf should be in the daemon's pool; got {counts:?}"
    );
    Ok(())
}

/// HODL receive: the daemon settles only after Alice's claim reveals the preimage.
#[tokio::test]
#[test_log::test]
async fn receive_over_real_channel() -> Result<()> {
    let stack = LightningStack::start().await?;
    setup_channel(&stack).await?;

    let alice_identity =
        spark::signer::derive_identity_public_key(stack.alice_signer.as_ref()).await?;
    let preimage = Preimage::try_from(vec![3u8; 32])?;
    let payment_hash = preimage.compute_hash();

    let request = stack
        .ssp_client()?
        .request_lightning_receive(RequestLightningReceiveInput {
            receiver_identity_pubkey: Some(alice_identity.serialize().to_vec().encode_hex()),
            amount_sats: 16_384,
            network: stack.alice_config.network.into(),
            payment_hash: payment_hash.to_byte_array().encode_hex(),
            description_hash: None,
            expiry_secs: Some(3600),
            memo: Some("itest".to_string()),
            include_spark_address: false,
            spark_invoice: None,
        })
        .await?;

    let invoice = stack
        .lightning_request(&request.id)
        .await?
        .receive
        .context("the daemon should have issued an invoice")?
        .encoded_invoice;

    let paid: api::Bolt11SendResponse = stack
        .counterparty
        .call_for_test(
            "Bolt11Send",
            &api::Bolt11SendRequest {
                invoice,
                amount_msat: None,
                route_parameters: None,
            },
        )
        .await?;
    info!("counterparty paid, payment {}", paid.payment_id);

    // Retries the claim rather than waiting on the record's transfer id, which the
    // daemon writes before the transfer reaches the operators.
    let claimed = wait_for_ok("Alice to be able to claim what she was fronted", || async {
        let record = stack
            .lightning_request(&request.id)
            .await?
            .receive
            .context("receive record")?;
        anyhow::ensure!(
            record.invoice_status == "pending",
            "the daemon settled before it had the preimage"
        );
        client_claim_htlc(&stack.alice_config, stack.alice_signer.clone(), &preimage).await
    })
    .await?;
    assert_eq!(claimed, 16_384, "Alice claims what she was paid");

    tokio::time::timeout(
        EVENT_SETTLE_TIMEOUT,
        wait_until("the daemon to settle the incoming payment", || async {
            Ok(stack
                .lightning_request(&request.id)
                .await?
                .receive
                .is_some_and(|r| r.invoice_status == "settled"))
        }),
    )
    .await
    .context("the daemon did not settle on the operators' event")??;
    Ok(())
}

#[tokio::test]
#[test_log::test]
async fn receive_normal_settles_via_swap() -> Result<()> {
    let stack = LightningStack::start().await?;
    setup_channel(&stack).await?;

    let alice_identity =
        spark::signer::derive_identity_public_key(stack.alice_signer.as_ref()).await?;
    let preimage = Preimage::try_from(vec![4u8; 32])?;
    let payment_hash = preimage.compute_hash();

    let request = stack
        .ssp_client()?
        .request_lightning_receive(RequestLightningReceiveInput {
            receiver_identity_pubkey: Some(alice_identity.serialize().to_vec().encode_hex()),
            amount_sats: 16_384,
            network: stack.alice_config.network.into(),
            payment_hash: payment_hash.to_byte_array().encode_hex(),
            description_hash: None,
            expiry_secs: Some(3600),
            memo: Some("itest".to_string()),
            include_spark_address: false,
            spark_invoice: None,
        })
        .await?;

    let invoice = stack
        .lightning_request(&request.id)
        .await?
        .receive
        .context("the daemon should have issued an invoice")?
        .encoded_invoice;

    client_store_preimage_shares(
        &stack.alice_config,
        stack.alice_signer.clone(),
        &preimage,
        invoice.clone(),
    )
    .await?;

    let paid: api::Bolt11SendResponse = stack
        .counterparty
        .call_for_test(
            "Bolt11Send",
            &api::Bolt11SendRequest {
                invoice,
                amount_msat: None,
                route_parameters: None,
            },
        )
        .await?;
    info!("counterparty paid, payment {}", paid.payment_id);

    wait_until("the daemon to settle from the swap", || async {
        Ok(stack
            .lightning_request(&request.id)
            .await?
            .receive
            .is_some_and(|r| r.invoice_status == "settled"))
    })
    .await?;

    let record = stack
        .lightning_request(&request.id)
        .await?
        .receive
        .context("receive record")?;
    assert!(
        record.has_preimage,
        "the daemon holds the preimage the swap returned it"
    );
    assert!(
        record.transfer_id.is_some(),
        "the daemon handed Alice her leaves"
    );
    Ok(())
}

#[tokio::test]
#[test_log::test]
async fn send_failure_does_not_claim() -> Result<()> {
    let stack = LightningStack::start().await?;
    setup_channel(&stack).await?;

    deposit_with_amount(&stack.alice, &stack.fixtures.bitcoind, 16384).await?;
    let leaf = stack.alice.list_leaves().await?.available[0].id.clone();

    let payment_hash = sha256::Hash::hash(&[4u8; 32]);
    let hold: api::Bolt11ReceiveForHashResponse = stack
        .counterparty
        .call_for_test(
            "Bolt11ReceiveForHash",
            &api::Bolt11ReceiveForHashRequest {
                amount_msat: Some(10_000_000),
                description: Some(types::Bolt11InvoiceDescription {
                    kind: Some(types::bolt11_invoice_description::Kind::Direct(
                        "itest-fail".to_string(),
                    )),
                }),
                expiry_secs: 3600,
                payment_hash: hex::encode(payment_hash.to_byte_array()),
                min_final_cltv_expiry_delta: None,
            },
        )
        .await?;

    let transfer = client_create_htlc(
        &stack.alice_config,
        stack.alice_signer.clone(),
        &stack.ssp_config,
        &stack.sspd.identity_public_key,
        &payment_hash,
        &leaf,
    )
    .await?;
    let id = request_send(&stack, &hold.invoice, 10_000, &transfer).await?;

    // Retried, since failing the hash has no effect on an HTLC that arrives
    // afterwards.
    wait_until("the daemon to record the payment as failed", || async {
        let _ = stack
            .counterparty
            .call_for_test::<_, api::Bolt11FailForHashResponse>(
                "Bolt11FailForHash",
                &api::Bolt11FailForHashRequest {
                    payment_hash: hex::encode(payment_hash.to_byte_array()),
                },
            )
            .await;
        Ok(send_record(&stack, &id).await?.payment_status == "failed")
    })
    .await?;

    let record = send_record(&stack, &id).await?;
    assert!(
        !record.leaves_claimed,
        "a failed payment must leave the user's leaf with the user"
    );
    assert!(
        !record.has_preimage,
        "the daemon never learned a preimage, because it never paid"
    );

    wait_until("the operators to return Alice's leaf", || async {
        stack.alice.sync().await?;
        Ok(stack
            .alice
            .list_leaves()
            .await?
            .available
            .iter()
            .any(|available| available.id == leaf))
    })
    .await?;
    Ok(())
}

#[tokio::test]
#[test_log::test]
async fn receive_never_reveal_returns_and_cancels() -> Result<()> {
    let stack = LightningStack::start().await?;
    setup_channel(&stack).await?;

    let alice_identity =
        spark::signer::derive_identity_public_key(stack.alice_signer.as_ref()).await?;
    let preimage = Preimage::try_from(vec![5u8; 32])?;
    let payment_hash = preimage.compute_hash();

    let request = stack
        .ssp_client()?
        .request_lightning_receive(RequestLightningReceiveInput {
            receiver_identity_pubkey: Some(alice_identity.serialize().to_vec().encode_hex()),
            amount_sats: 16_384,
            network: stack.alice_config.network.into(),
            payment_hash: payment_hash.to_byte_array().encode_hex(),
            description_hash: None,
            expiry_secs: Some(3600),
            memo: Some("itest-no-reveal".to_string()),
            include_spark_address: false,
            spark_invoice: None,
        })
        .await?;

    let invoice = stack
        .lightning_request(&request.id)
        .await?
        .receive
        .context("the daemon should have issued an invoice")?
        .encoded_invoice;

    let _: api::Bolt11SendResponse = stack
        .counterparty
        .call_for_test(
            "Bolt11Send",
            &api::Bolt11SendRequest {
                invoice,
                amount_msat: None,
                route_parameters: None,
            },
        )
        .await?;

    wait_until("the daemon to give up and cancel", || async {
        Ok(stack
            .lightning_request(&request.id)
            .await?
            .receive
            .is_some_and(|r| r.invoice_status == "cancelled"))
    })
    .await?;

    let record = stack
        .lightning_request(&request.id)
        .await?
        .receive
        .context("receive record")?;
    assert!(
        !record.has_preimage,
        "the daemon never learned the preimage, so it settled nothing"
    );
    Ok(())
}

/// An amountless invoice paid while the daemon is stopped: ldk-server does not
/// replay events, so the held payment and its amount come from its payment list.
#[tokio::test]
#[test_log::test]
async fn receive_recovers_after_a_restart() -> Result<()> {
    let mut stack = LightningStack::start().await?;
    setup_channel(&stack).await?;

    let alice_identity =
        spark::signer::derive_identity_public_key(stack.alice_signer.as_ref()).await?;
    let preimage = Preimage::try_from(vec![6u8; 32])?;
    let payment_hash = preimage.compute_hash();

    let request = stack
        .ssp_client()?
        .request_lightning_receive(RequestLightningReceiveInput {
            receiver_identity_pubkey: Some(alice_identity.serialize().to_vec().encode_hex()),
            amount_sats: 0,
            network: stack.alice_config.network.into(),
            payment_hash: payment_hash.to_byte_array().encode_hex(),
            description_hash: None,
            expiry_secs: Some(3600),
            memo: Some("itest-restart".to_string()),
            include_spark_address: false,
            spark_invoice: None,
        })
        .await?;

    let invoice = stack
        .lightning_request(&request.id)
        .await?
        .receive
        .context("the daemon should have issued an invoice")?
        .encoded_invoice;

    stack.sspd.restart_stopped().await?;

    let _: api::Bolt11SendResponse = stack
        .counterparty
        .call_for_test(
            "Bolt11Send",
            &api::Bolt11SendRequest {
                invoice,
                amount_msat: Some(16_384_000),
                route_parameters: None,
            },
        )
        .await?;
    wait_for_ok("the counterparty's HTLC to be held", || async {
        held_htlc_deadline(&stack.ssp_node, &payment_hash.to_byte_array())
            .await?
            .context("not held yet")
    })
    .await?;

    stack.sspd.start_again().await?;
    let claimed = wait_for_ok("the daemon to front Alice her leaves anyway", || async {
        client_claim_htlc(&stack.alice_config, stack.alice_signer.clone(), &preimage).await
    })
    .await?;
    assert_eq!(claimed, 16_384, "Alice is paid what the payer sent");

    wait_until("the daemon to settle the recovered payment", || async {
        Ok(stack
            .lightning_request(&request.id)
            .await?
            .receive
            .is_some_and(|r| r.invoice_status == "settled"))
    })
    .await?;
    Ok(())
}

#[tokio::test]
#[test_log::test]
async fn receive_refuses_when_deadline_too_close() -> Result<()> {
    let mut stack = LightningStack::start().await?;
    setup_channel(&stack).await?;

    let alice_identity =
        spark::signer::derive_identity_public_key(stack.alice_signer.as_ref()).await?;
    let preimage = Preimage::try_from(vec![7u8; 32])?;
    let payment_hash = preimage.compute_hash();

    let request = stack
        .ssp_client()?
        .request_lightning_receive(RequestLightningReceiveInput {
            receiver_identity_pubkey: Some(alice_identity.serialize().to_vec().encode_hex()),
            amount_sats: 16_384,
            network: stack.alice_config.network.into(),
            payment_hash: payment_hash.to_byte_array().encode_hex(),
            description_hash: None,
            expiry_secs: Some(3600),
            memo: Some("itest-deadline".to_string()),
            include_spark_address: false,
            spark_invoice: None,
        })
        .await?;

    let invoice = stack
        .lightning_request(&request.id)
        .await?
        .receive
        .context("the daemon should have issued an invoice")?
        .encoded_invoice;

    // Stopped before the payment, since a running daemon fronts leaves as soon as
    // the HTLC is held.
    stack.sspd.restart_stopped().await?;

    let _: api::Bolt11SendResponse = stack
        .counterparty
        .call_for_test(
            "Bolt11Send",
            &api::Bolt11SendRequest {
                invoice,
                amount_msat: None,
                route_parameters: None,
            },
        )
        .await?;

    let claim_deadline = wait_for_ok("the counterparty's HTLC to be held", || async {
        held_htlc_deadline(&stack.ssp_node, &payment_hash.to_byte_array())
            .await?
            .context("not held yet")
    })
    .await?;
    let margin = min_blocks_until_deadline(Duration::from_secs(
        spark_itest::fixtures::sspd::RECEIVE_LEAF_TRANSFER_EXPIRY_SECS,
    ));
    let target = claim_deadline.saturating_sub(margin.saturating_sub(2));
    // Two blocks at a time, checked against the node's height, which lags bitcoind:
    // the tip has to land inside the margin but short of the claim deadline.
    wait_until("the tip to reach the margin", || async {
        if stack.ssp_node.current_block_height().await? >= target {
            return Ok(true);
        }
        stack.fixtures.bitcoind.generate_blocks(2).await?;
        Ok(false)
    })
    .await?;

    stack.sspd.start_again().await?;
    wait_until("the daemon to refuse and refund", || async {
        Ok(stack
            .lightning_request(&request.id)
            .await?
            .receive
            .is_some_and(|r| r.invoice_status == "cancelled"))
    })
    .await?;

    let record = stack
        .lightning_request(&request.id)
        .await?
        .receive
        .context("receive record")?;
    assert!(
        record.transfer_id.is_none(),
        "the daemon must not have fronted leaves it could not settle for"
    );
    Ok(())
}

#[tokio::test]
#[test_log::test]
async fn send_never_routes_above_the_collected_fee() -> Result<()> {
    let stack = LightningStack::start().await?;
    let bitcoind = &stack.fixtures.bitcoind;

    let dest_ldk = LdkServerFixture::start(&stack.fixtures.fixture_id, bitcoind, "dest").await?;
    let dest_node = LdkServerNode::new(
        dest_ldk.base_url.clone(),
        dest_ldk.api_key.clone(),
        &dest_ldk.cert_pem,
        Wakeup::new(),
        Wakeup::new(),
    )?;

    setup_channel(&stack).await?;
    // A forwarding fee applies to the channel a node forwards out over, so the
    // router's fee is set on its channel to the destination.
    open_channel_between(
        &stack.counterparty,
        &dest_node,
        &dest_ldk.peer_address(),
        bitcoind,
        Some(types::ChannelConfig {
            forwarding_fee_base_msat: Some(5_000_000),
            forwarding_fee_proportional_millionths: Some(0),
            ..Default::default()
        }),
    )
    .await?;

    // The route's fee budget is what the leaf commits beyond the send: 384 sats.
    deposit_with_amount(&stack.alice, bitcoind, 16384).await?;
    let leaf = stack.alice.list_leaves().await?.available[0].id.clone();

    let invoice = {
        let resp: api::Bolt11ReceiveResponse = dest_node
            .call_for_test(
                "Bolt11Receive",
                &api::Bolt11ReceiveRequest {
                    amount_msat: Some(16_000_000),
                    description: Some(types::Bolt11InvoiceDescription {
                        kind: Some(types::bolt11_invoice_description::Kind::Direct(
                            "itest-fee".to_string(),
                        )),
                    }),
                    expiry_secs: 3600,
                },
            )
            .await?;
        resp.invoice
    };
    let decoded = stack.ssp_node.decode_invoice(&invoice).await?;
    let payment_hash = sha256::Hash::from_byte_array(decoded.payment_hash);
    let transfer = client_create_htlc(
        &stack.alice_config,
        stack.alice_signer.clone(),
        &stack.ssp_config,
        &stack.sspd.identity_public_key,
        &payment_hash,
        &leaf,
    )
    .await?;

    let id = request_send(&stack, &invoice, 16_000, &transfer).await?;
    wait_until("the daemon to give up on the route", || async {
        Ok(send_record(&stack, &id).await?.payment_status == "failed")
    })
    .await?;

    let record = send_record(&stack, &id).await?;
    assert!(
        !record.leaves_claimed,
        "a route it would not pay for leaves the user's leaf alone"
    );
    assert!(
        !record.has_preimage,
        "the daemon never paid, so it never learned a preimage"
    );
    Ok(())
}
