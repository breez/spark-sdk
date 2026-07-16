use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};
use bitcoin::{
    Address, Amount, CompressedPublicKey, OutPoint, Psbt, Transaction, TxOut, Witness,
    hashes::Hash as _,
    key::{Secp256k1, TapTweak as _},
    secp256k1::SecretKey,
    sighash::{self, SighashCache},
};
use rand::Rng;
use rstest::*;
use spark_mysql::{
    MysqlSessionStore, MysqlTokenStore, MysqlTreeStore, default_mysql_storage_config,
};
use spark_postgres::{
    PostgresSessionStore, PostgresTokenStore, PostgresTreeStore, default_postgres_storage_config,
};
use spark_wallet::{
    DefaultSigner, Network, SessionStore, Signer, SparkSigner, SparkSignerAdapter, SparkWallet,
    SparkWalletConfig, WalletBuilder, WalletEvent, is_ephemeral_anchor_output,
};
use tokio::sync::broadcast::Receiver;
use tracing::{debug, info};

use crate::backend::Backend;
use crate::fixtures::{
    bitcoind::BitcoindFixture,
    setup::{TestFixtures, create_test_signer_alice, create_test_signer_bob},
};

/// Builds a `SparkWallet` whose session, tree, and token-output stores are
/// backed by the resolved `Backend`. For `Backend::InMemory` this is
/// equivalent to `SparkWallet::connect(config, signer)`; for the SQL variants
/// all three stores share the connection string (so they live in one
/// database) and are scoped per-wallet by the signer's identity pubkey.
///
/// Background processing is **not** started. Callers must `subscribe_events()`
/// first and then call `wallet.start_background_processing()`, otherwise the
/// wallet's first `WalletEvent::Synced` fires before any listener exists and
/// is dropped.
pub async fn build_test_wallet(
    config: SparkWalletConfig,
    signer: Arc<dyn Signer>,
    backend: &Backend,
) -> Result<SparkWallet> {
    let spark_signer = Arc::new(SparkSignerAdapter::new(signer.clone()));
    let mut builder = WalletBuilder::new(config, spark_signer.clone());
    match backend {
        Backend::InMemory => {}
        Backend::Postgres(conn_str) => {
            let identity = spark_signer.get_identity_public_key().await?.serialize();
            let pg_config = default_postgres_storage_config(conn_str.clone());
            let session = PostgresSessionStore::from_config(pg_config.clone(), &identity).await?;
            let tree = PostgresTreeStore::from_config(pg_config.clone(), &identity).await?;
            let token = PostgresTokenStore::from_config(pg_config, &identity).await?;
            builder = builder
                .with_session_store(Arc::new(session))
                .with_tree_store(Arc::new(tree))
                .with_token_output_store(Arc::new(token));
        }
        Backend::Mysql(conn_str) => {
            let identity = spark_signer.get_identity_public_key().await?.serialize();
            let my_config = default_mysql_storage_config(conn_str.clone());
            let session = MysqlSessionStore::from_config(my_config.clone(), &identity).await?;
            let tree = MysqlTreeStore::from_config(my_config.clone(), &identity).await?;
            let token = MysqlTokenStore::from_config(my_config, &identity).await?;
            builder = builder
                .with_session_store(Arc::new(session))
                .with_tree_store(Arc::new(tree))
                .with_token_output_store(Arc::new(token));
        }
    }
    Ok(builder.build().await?)
}

pub async fn wait_for_event<F>(
    event_rx: &mut Receiver<WalletEvent>,
    timeout_secs: u64,
    event_name: &str,
    mut matcher: F,
) -> Result<WalletEvent>
where
    F: FnMut(WalletEvent) -> Result<Option<WalletEvent>>,
{
    let timeout = tokio::time::Duration::from_secs(timeout_secs);
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            anyhow::bail!(
                "Timeout waiting for {} event after {} seconds",
                event_name,
                timeout_secs
            );
        }

        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Ok(event)) => {
                match matcher(event) {
                    Ok(Some(result)) => return Ok(result),
                    Ok(None) => {
                        // Not the event we're looking for, keep waiting
                        continue;
                    }
                    Err(e) => {
                        // Matcher returned an error (e.g., failure event)
                        return Err(e);
                    }
                }
            }
            Ok(Err(_)) => {
                anyhow::bail!("Event channel closed unexpectedly");
            }
            Err(_) => {
                anyhow::bail!(
                    "Timeout waiting for {} event after {} seconds",
                    event_name,
                    timeout_secs
                );
            }
        }
    }
}

/// Create a wallet connected to deployed regtest operators with a random seed.
/// Waits for the wallet to sync before returning.
///
/// # Returns
/// A tuple of (SparkWallet, event listener)
pub async fn create_regtest_wallet() -> Result<(SparkWallet, Receiver<WalletEvent>)> {
    let mut config = SparkWalletConfig::default_config(Network::Regtest);
    config.leaf_auto_optimize_enabled = false;

    let mut seed = [0u8; 32];
    rand::thread_rng().fill(&mut seed);
    let signer = Arc::new(DefaultSigner::new(&seed, Network::Regtest)?);

    info!("Connecting wallet to deployed regtest operators...");
    let backend = crate::backend::resolve_backend().await?;
    let wallet = build_test_wallet(config, signer, &backend).await?;
    let mut listener = wallet.subscribe_events();
    wallet.start_background_processing().await;

    // Wait for initial sync
    wait_for_event(&mut listener, 60, "Synced", |e| match e {
        WalletEvent::Synced => Ok(Some(e)),
        _ => Ok(None),
    })
    .await?;

    info!("Wallet synced successfully");
    Ok((wallet, listener))
}

/// Like [`create_regtest_wallet`] but installs a caller-provided session store,
/// so tests can exercise the operator auth-token behavior (the force-refresh
/// self-heal). Tree and token-output stores stay in memory.
pub async fn create_regtest_wallet_with_session_store(
    session_store: Arc<dyn SessionStore>,
) -> Result<(SparkWallet, Receiver<WalletEvent>)> {
    let mut config = SparkWalletConfig::default_config(Network::Regtest);
    config.leaf_auto_optimize_enabled = false;

    let mut seed = [0u8; 32];
    rand::thread_rng().fill(&mut seed);
    let signer = Arc::new(DefaultSigner::new(&seed, Network::Regtest)?);
    let spark_signer = Arc::new(SparkSignerAdapter::new(signer));

    info!("Connecting wallet to deployed regtest operators (custom session store)...");
    let wallet = WalletBuilder::new(config, spark_signer)
        .with_session_store(session_store)
        .build()
        .await?;
    let mut listener = wallet.subscribe_events();
    wallet.start_background_processing().await;

    wait_for_event(&mut listener, 60, "Synced", |e| match e {
        WalletEvent::Synced => Ok(Some(e)),
        _ => Ok(None),
    })
    .await?;

    info!("Wallet synced successfully");
    Ok((wallet, listener))
}

// ============================================================================
// Local Docker test fixtures
// ============================================================================

#[fixture]
pub async fn test_fixtures() -> TestFixtures {
    TestFixtures::new()
        .await
        .expect("Failed to initialize test fixtures")
}

pub struct WalletsFixture {
    pub fixtures: TestFixtures,
    pub alice_wallet: SparkWallet,
    pub bob_wallet: SparkWallet,
}

#[fixture]
pub async fn wallets(#[future] test_fixtures: TestFixtures) -> WalletsFixture {
    let fixtures = test_fixtures.await;
    let config = fixtures
        .create_wallet_config()
        .await
        .expect("failed to create wallet config");
    let signer = create_test_signer_alice();

    let alice_backend = crate::backend::resolve_backend()
        .await
        .expect("failed to resolve backend for alice wallet");
    let alice_wallet = build_test_wallet(config.clone(), Arc::new(signer), &alice_backend)
        .await
        .expect("Failed to connect alice wallet");
    let mut alice_listener = alice_wallet.subscribe_events();
    alice_wallet.start_background_processing().await;

    let bob_backend = crate::backend::resolve_backend()
        .await
        .expect("failed to resolve backend for bob wallet");
    let bob_wallet = build_test_wallet(config, Arc::new(create_test_signer_bob()), &bob_backend)
        .await
        .expect("Failed to connect bob wallet");
    let mut bob_listener = bob_wallet.subscribe_events();
    bob_wallet.start_background_processing().await;

    loop {
        let event = alice_listener
            .recv()
            .await
            .expect("Failed to receive alice wallet event");
        info!("Alice wallet event: {:?}", event);
        if matches!(event, WalletEvent::Synced) {
            break;
        }
    }
    loop {
        let event = bob_listener
            .recv()
            .await
            .expect("Failed to receive bob wallet event");
        info!("Bob wallet event: {:?}", event);
        if matches!(event, WalletEvent::Synced) {
            break;
        }
    }

    WalletsFixture {
        fixtures,
        alice_wallet,
        bob_wallet,
    }
}

pub async fn deposit_to_wallet(wallet: &SparkWallet, bitcoind: &BitcoindFixture) -> Result<()> {
    // Generate a non-static deposit address
    let deposit_address = wallet.generate_deposit_address().await?.address;
    info!("Generated deposit address: {}", deposit_address);

    // Fund the deposit address with a certain amount
    let deposit_amount = Amount::from_sat(100_000); // 0.001 BTC
    let txid = bitcoind
        .fund_address(&deposit_address, deposit_amount)
        .await?;
    info!(
        "Funded deposit address with {}, txid: {}",
        deposit_amount, txid
    );

    // Get the transaction to claim
    let tx = bitcoind.get_transaction(&txid).await?;
    info!("Got transaction: {:?}", tx);

    // Find the output index for our address
    let mut output_index = None;
    for (vout, output) in tx.output.iter().enumerate() {
        if let Ok(address) =
            bitcoin::Address::from_script(&output.script_pubkey, bitcoin::Network::Regtest)
            && address == deposit_address
        {
            output_index = Some(vout as u32);
            break;
        }
    }

    let vout = output_index.expect("Could not find deposit address in transaction outputs");
    info!("Found deposit output at index: {}", vout);

    let mut listener = wallet.subscribe_events();
    let leaves = wallet.claim_deposit(tx, vout).await?;
    info!("Claimed deposit, got leaves: {:?}", leaves);

    // Mine a block to confirm the transaction
    bitcoind.generate_blocks(1).await?;
    bitcoind.wait_for_tx_confirmation(&txid, 1).await?;
    info!("Transaction confirmed");

    // Wait for the deposit confirmation event from the SO.
    loop {
        let event: WalletEvent = listener
            .recv()
            .await
            .expect("Failed to receive wallet event");
        match event {
            WalletEvent::DepositConfirmed(_) => {
                info!("Received deposit confirmed event");
                break;
            }
            _ => debug!("Received other event: {:?}", event),
        }
    }

    // Check that balance increased by the deposit amount
    let final_balance = wallet.get_balance().await?;
    info!("Final balance: {}", final_balance);

    // The deposited amount should now be reflected in the balance
    assert_eq!(
        final_balance,
        deposit_amount.to_sat(),
        "Balance should be the deposit amount"
    );
    Ok(())
}

/// Non-static deposit a specific amount to a wallet.
/// Similar to deposit_to_wallet but allows specifying the amount.
pub async fn deposit_with_amount(
    wallet: &SparkWallet,
    bitcoind: &BitcoindFixture,
    amount_sats: u64,
) -> Result<()> {
    // Generate a deposit address
    let deposit_address = wallet.generate_deposit_address().await?.address;
    info!("Generated deposit address: {}", deposit_address);

    // Fund the deposit address with the specified amount
    let deposit_amount = Amount::from_sat(amount_sats);
    let txid = bitcoind
        .fund_address(&deposit_address, deposit_amount)
        .await?;
    info!(
        "Funded deposit address with {}, txid: {}",
        deposit_amount, txid
    );

    // Get the transaction to claim
    let tx = bitcoind.get_transaction(&txid).await?;
    info!("Got transaction: {:?}", tx);

    // Find the output index for our address
    let mut output_index = None;
    for (vout, output) in tx.output.iter().enumerate() {
        if let Ok(address) =
            bitcoin::Address::from_script(&output.script_pubkey, bitcoin::Network::Regtest)
            && address == deposit_address
        {
            output_index = Some(vout as u32);
            break;
        }
    }

    let vout = output_index.expect("Could not find deposit address in transaction outputs");
    info!("Found deposit output at index: {}", vout);

    let mut listener = wallet.subscribe_events();
    let leaves = wallet.claim_deposit(tx, vout).await?;
    info!("Claimed deposit, got leaves: {:?}", leaves);

    // Mine a block to confirm the transaction
    bitcoind.generate_blocks(1).await?;
    bitcoind.wait_for_tx_confirmation(&txid, 1).await?;
    info!("Transaction confirmed");

    // Wait for the deposit confirmation event from the SO.
    loop {
        let event: WalletEvent = listener
            .recv()
            .await
            .expect("Failed to receive wallet event");
        match event {
            WalletEvent::DepositConfirmed(_) => {
                info!("Received deposit confirmed event");
                break;
            }
            _ => debug!("Received other event: {:?}", event),
        }
    }

    Ok(())
}

/// Polls a condition function every 50ms until it returns true or the timeout is reached.
/// Returns Ok(()) if the condition was met, or an error if the timeout was reached.
pub async fn wait_for<F, Fut>(condition: F, timeout_secs: u64, description: &str) -> Result<()>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let timeout = Duration::from_secs(timeout_secs);
    let poll_interval = Duration::from_millis(50);
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        if condition().await {
            return Ok(());
        }

        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "Timeout waiting for condition '{}' after {} seconds",
                description,
                timeout_secs
            );
        }

        tokio::time::sleep(poll_interval).await;
    }
}

// ---------------------------------------------------------------------------
// Unilateral exit test helpers: fund regtest CPFP UTXOs, sign CPFP PSBTs, and
// submit parent+child packages.
// ---------------------------------------------------------------------------

/// A funded regtest UTXO ready for use as a CPFP input.
pub struct FundedUtxo {
    pub secret_key: SecretKey,
    pub outpoint: OutPoint,
    pub witness_utxo: TxOut,
    pub address: Address,
}

/// Fund a new P2TR address from bitcoind and return the UTXO details.
pub async fn fund_p2tr_utxo(bitcoind: &BitcoindFixture, amount: Amount) -> Result<FundedUtxo> {
    fund_p2tr_utxo_with_key(bitcoind, amount, &SecretKey::new(&mut rand::thread_rng())).await
}

/// Fund a new P2TR address but leave the funding transaction unconfirmed in the
/// mempool (no block mined).
pub async fn fund_p2tr_utxo_unmined(
    bitcoind: &BitcoindFixture,
    amount: Amount,
) -> Result<FundedUtxo> {
    let secp = Secp256k1::new();
    let secret_key = SecretKey::new(&mut rand::thread_rng());
    let pubkey = secret_key.public_key(&secp);
    let (xonly, _) = pubkey.x_only_public_key();
    let address = Address::p2tr(&secp, xonly, None, bitcoin::Network::Regtest);

    let txid = bitcoind.fund_address(&address, amount).await?;
    let tx = bitcoind.get_transaction(&txid).await?;
    let vout = tx
        .output
        .iter()
        .position(|o| o.script_pubkey == address.script_pubkey())
        .expect("P2TR output not found") as u32;

    Ok(FundedUtxo {
        secret_key,
        outpoint: OutPoint { txid, vout },
        witness_utxo: TxOut {
            value: amount,
            script_pubkey: address.script_pubkey(),
        },
        address,
    })
}

/// Fund a P2TR address for a caller-supplied key.
pub async fn fund_p2tr_utxo_with_key(
    bitcoind: &BitcoindFixture,
    amount: Amount,
    secret_key: &SecretKey,
) -> Result<FundedUtxo> {
    let secp = Secp256k1::new();
    let pubkey = secret_key.public_key(&secp);
    let (xonly, _) = pubkey.x_only_public_key();
    let address = Address::p2tr(&secp, xonly, None, bitcoin::Network::Regtest);

    let txid = bitcoind.fund_address(&address, amount).await?;
    bitcoind.generate_blocks(1).await?;
    bitcoind.wait_for_tx_confirmation(&txid, 1).await?;

    let tx = bitcoind.get_transaction(&txid).await?;
    let vout = tx
        .output
        .iter()
        .position(|o| o.script_pubkey == address.script_pubkey())
        .expect("P2TR output not found") as u32;

    Ok(FundedUtxo {
        secret_key: *secret_key,
        outpoint: OutPoint { txid, vout },
        witness_utxo: TxOut {
            value: amount,
            script_pubkey: address.script_pubkey(),
        },
        address,
    })
}

/// Fund a new P2WPKH address from bitcoind and return the UTXO details.
pub async fn fund_p2wpkh_utxo(bitcoind: &BitcoindFixture, amount: Amount) -> Result<FundedUtxo> {
    fund_p2wpkh_utxo_with_key(bitcoind, amount, &SecretKey::new(&mut rand::thread_rng())).await
}

/// Fund a P2WPKH address for a caller-supplied key.
pub async fn fund_p2wpkh_utxo_with_key(
    bitcoind: &BitcoindFixture,
    amount: Amount,
    secret_key: &SecretKey,
) -> Result<FundedUtxo> {
    let secp = Secp256k1::new();
    let pubkey = secret_key.public_key(&secp);
    let compressed = CompressedPublicKey(pubkey);
    let address = Address::p2wpkh(&compressed, bitcoin::Network::Regtest);

    let txid = bitcoind.fund_address(&address, amount).await?;
    bitcoind.generate_blocks(1).await?;
    bitcoind.wait_for_tx_confirmation(&txid, 1).await?;

    let tx = bitcoind.get_transaction(&txid).await?;
    let vout = tx
        .output
        .iter()
        .position(|o| o.script_pubkey == address.script_pubkey())
        .expect("P2WPKH output not found") as u32;

    Ok(FundedUtxo {
        secret_key: *secret_key,
        outpoint: OutPoint { txid, vout },
        witness_utxo: TxOut {
            value: amount,
            script_pubkey: address.script_pubkey(),
        },
        address,
    })
}

/// Finalize all ephemeral anchor inputs in a PSBT with an empty witness.
fn finalize_anchor_inputs(psbt: &mut Psbt) {
    for input in &mut psbt.inputs {
        if let Some(ref tx_out) = input.witness_utxo
            && is_ephemeral_anchor_output(tx_out)
        {
            input.final_script_witness = Some(Witness::new());
        }
    }
}

/// Sign a CPFP PSBT with P2TR external inputs, finalizing anchor + P2TR inputs.
pub fn sign_cpfp_psbt_p2tr(psbt: &Psbt, secret_key: &SecretKey) -> Result<Transaction> {
    let mut psbt = psbt.clone();
    finalize_anchor_inputs(&mut psbt);

    let secp = Secp256k1::new();
    let keypair = bitcoin::key::Keypair::from_secret_key(&secp, secret_key)
        .tap_tweak(&secp, None)
        .to_keypair();

    let prevouts: Vec<TxOut> = psbt
        .inputs
        .iter()
        .map(|i| i.witness_utxo.clone().unwrap_or(TxOut::NULL))
        .collect();
    let prevouts_ref = sighash::Prevouts::All(&prevouts);

    let taproot_indices: Vec<usize> = psbt
        .inputs
        .iter()
        .enumerate()
        .filter(|(_, i)| {
            i.final_script_witness.is_none()
                && i.witness_utxo
                    .as_ref()
                    .is_some_and(|o| o.script_pubkey.is_p2tr())
        })
        .map(|(idx, _)| idx)
        .collect();

    let mut cache = SighashCache::new(&psbt.unsigned_tx);
    for i in taproot_indices {
        let sighash = cache.taproot_key_spend_signature_hash(
            i,
            &prevouts_ref,
            sighash::TapSighashType::Default,
        )?;
        let msg = bitcoin::secp256k1::Message::from_digest(sighash.to_byte_array());
        let schnorr_sig = secp.sign_schnorr_no_aux_rand(&msg, &keypair);
        let tap_sig = bitcoin::taproot::Signature {
            signature: schnorr_sig,
            sighash_type: sighash::TapSighashType::Default,
        };
        let mut witness = Witness::new();
        witness.push(tap_sig.to_vec());
        psbt.inputs[i].final_script_witness = Some(witness);
    }

    Ok(psbt.extract_tx_unchecked_fee_rate())
}

/// Submit a signed parent+child package, retrying once after mining the
/// required blocks if the first attempt fails on a BIP68 CSV timelock.
pub async fn submit_package_with_csv_retry(
    bitcoind: &BitcoindFixture,
    parent: &Transaction,
    child: &Transaction,
) -> Result<()> {
    let result = bitcoind.submit_package(&[parent, child]).await?;
    let pkg_msg = result
        .get("package_msg")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    if pkg_msg == "success" {
        return Ok(());
    }

    let has_bip68_error = result
        .get("tx-results")
        .and_then(|v| v.as_object())
        .is_some_and(|txs| {
            txs.values().any(|d| {
                d.get("error")
                    .and_then(|v| v.as_str())
                    .is_some_and(|e| e.contains("non-BIP68-final"))
            })
        });
    if !has_bip68_error {
        bail!(
            "Package failed: {}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );
    }
    // BIP68: every input's relative lock must be satisfied, so mine enough
    // blocks to cover the max across all block-based CSVs.
    let csv_blocks: u32 = parent
        .input
        .iter()
        .filter_map(|i| match i.sequence.to_relative_lock_time()? {
            bitcoin::relative::LockTime::Blocks(h) => Some(u32::from(h.value())),
            bitcoin::relative::LockTime::Time(_) => None,
        })
        .max()
        .unwrap_or(0);
    bitcoind.generate_blocks(csv_blocks.into()).await?;

    let retry = bitcoind.submit_package(&[parent, child]).await?;
    let retry_msg = retry
        .get("package_msg")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    if retry_msg != "success" {
        bail!("Package still failed after CSV: {retry:?}");
    }
    Ok(())
}
