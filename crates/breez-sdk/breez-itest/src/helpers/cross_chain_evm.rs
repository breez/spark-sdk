//! EVM-side helpers for the mainnet cross-chain send itests: derive the
//! deterministic recipient from the test mnemonic, and read ERC-20 balances over
//! JSON-RPC so a cross-chain send can be verified independently of the SDK's own
//! status reporting.
//!
//! The recipient is derived at the standard Ethereum path `m/44'/60'/0'/0/0` with
//! an empty BIP-39 passphrase, from the same mnemonic the test account ("Alice")
//! uses. Importing that mnemonic into any EVM wallet at the default path
//! reproduces this address and its private key, so funds delivered here are
//! recoverable (no automated sweep: the SDK has no Spark-inbound path from EVM).

use std::time::{Duration, Instant};

use alloy_primitives::{Address, U256};
use alloy_signer_local::{MnemonicBuilder, PrivateKeySigner, coins_bip39::English};
use anyhow::{Result, anyhow, bail};
use serde_json::json;
use tracing::{debug, info};

/// Standard Ethereum BIP-44 derivation path (first account, first address). The
/// same address every mainstream EVM wallet shows for a freshly imported seed.
const EVM_DERIVATION_PATH: &str = "m/44'/60'/0'/0/0";

/// ERC-20 `balanceOf(address)` selector: first 4 bytes of `keccak256` of the
/// signature.
const ERC20_BALANCE_OF_SELECTOR: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];

/// Poll interval while waiting on an external-chain balance to change. Kept
/// gentle so public RPC endpoints don't rate-limit during the long settlement
/// window.
const EVM_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Shared HTTP client, built once. `wait_for_evm_balance_increase` reads the
/// balance every few seconds over a multi-minute window, so a fresh client (and
/// its connection pool / TLS setup) per call would be wasteful.
fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    // Test-only helper talking to a local EVM node; no SDK proxy applies.
    #[allow(clippy::disallowed_methods)]
    CLIENT.get_or_init(reqwest::Client::new)
}

/// Derive the deterministic EVM recipient (checksummed address string + signer)
/// from the mainnet test mnemonic. See the module docs for the path / recovery.
///
/// The signer is returned alongside the address so a future recovery step (or the
/// deferred cross-chain receive tests) can sign outbound transfers without
/// re-deriving.
pub fn mainnet_evm_recipient(mnemonic: &str) -> Result<(String, PrivateKeySigner)> {
    let signer = MnemonicBuilder::<English>::default()
        .phrase(mnemonic)
        .derivation_path(EVM_DERIVATION_PATH)?
        .build()?;
    Ok((signer.address().to_string(), signer))
}

/// Map a route `chain` string to a public JSON-RPC URL. The primary target is
/// Arbitrum One. Returns `None` for chains we have no endpoint for, so callers
/// skip rather than fail. A single `balanceOf` read per test is well within
/// public-endpoint limits, so there's no override knob.
pub fn evm_rpc_url(chain: &str) -> Option<String> {
    let url = match chain.to_lowercase().as_str() {
        "arbitrum" | "arbitrum one" | "arbitrum-one" => "https://arb1.arbitrum.io/rpc",
        "base" => "https://mainnet.base.org",
        "polygon" | "polygon pos" => "https://polygon-rpc.com",
        _ => return None,
    };
    Some(url.to_string())
}

/// Read an ERC-20 `balanceOf(holder)` via a single `eth_call`, returning the
/// balance in the token's base units. Stateless: no provider/contract crates,
/// just a JSON-RPC POST.
pub async fn evm_erc20_balance(rpc_url: &str, token_contract: &str, holder: &str) -> Result<u128> {
    let token: Address = token_contract
        .parse()
        .map_err(|e| anyhow!("invalid token contract {token_contract}: {e}"))?;
    let holder: Address = holder
        .parse()
        .map_err(|e| anyhow!("invalid holder address {holder}: {e}"))?;

    // calldata = selector ++ 32-byte left-padded holder address
    let mut data = Vec::with_capacity(4 + 32);
    data.extend_from_slice(&ERC20_BALANCE_OF_SELECTOR);
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(holder.as_slice());

    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_call",
        "params": [
            { "to": token.to_string(), "data": format!("0x{}", hex::encode(&data)) },
            "latest",
        ],
    });

    let resp: serde_json::Value = http_client()
        .post(rpc_url)
        .json(&body)
        .send()
        .await?
        .json()
        .await?;

    if let Some(err) = resp.get("error") {
        bail!("eth_call error from {rpc_url}: {err}");
    }
    let result = resp
        .get("result")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("eth_call returned no result: {resp}"))?;
    let value = U256::from_str_radix(result.trim_start_matches("0x"), 16)
        .map_err(|e| anyhow!("eth_call: bad hex result {result}: {e}"))?;
    u128::try_from(value).map_err(|_| anyhow!("ERC-20 balance exceeds u128: {value}"))
}

/// Poll `evm_erc20_balance` until the holder's balance is at least `baseline +
/// min_delta`, or `timeout_secs` elapses. Returns the observed balance on
/// success. Transient RPC errors are retried (not fatal) until the deadline.
pub async fn wait_for_evm_balance_increase(
    rpc_url: &str,
    token_contract: &str,
    holder: &str,
    baseline: u128,
    min_delta: u128,
    timeout_secs: u64,
) -> Result<u128> {
    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_secs);
    let target = baseline.saturating_add(min_delta);
    loop {
        match evm_erc20_balance(rpc_url, token_contract, holder).await {
            Ok(balance) if balance >= target => return Ok(balance),
            Ok(balance) => {
                debug!("EVM balance {balance} (baseline {baseline}, need >= {target}); waiting...");
            }
            Err(e) => debug!("EVM balance read failed (retrying): {e:#}"),
        }
        if start.elapsed() >= timeout {
            bail!(
                "timeout after {timeout_secs}s waiting for {holder} balance to reach {target} \
                 (baseline {baseline}, +{min_delta}) for token {token_contract}"
            );
        }
        tokio::time::sleep(EVM_POLL_INTERVAL).await;
    }
}

/// `keccak256("Transfer(address,address,uint256)")`, `topics[0]` of every
/// ERC-20 transfer log.
const ERC20_TRANSFER_TOPIC: &str =
    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

/// Total amount of `token_contract` credited to `recipient` by `tx_hash`, read
/// from the transaction's own receipt logs. `None` while the node has not
/// indexed the transaction yet; `Some(0)` if it is mined but credits the
/// recipient nothing.
///
/// This is what turns a provider-reported settlement hash into evidence: it
/// proves the named transaction exists on the destination chain, succeeded, and
/// moved the token to the address we asked for.
pub async fn evm_erc20_credited_by_tx(
    rpc_url: &str,
    tx_hash: &str,
    token_contract: &str,
    recipient: &str,
) -> Result<Option<u128>> {
    let token: Address = token_contract
        .parse()
        .map_err(|e| anyhow!("invalid token contract {token_contract}: {e}"))?;
    let recipient: Address = recipient
        .parse()
        .map_err(|e| anyhow!("invalid recipient address {recipient}: {e}"))?;

    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_getTransactionReceipt",
        "params": [tx_hash],
    });
    let resp: serde_json::Value = http_client()
        .post(rpc_url)
        .json(&body)
        .send()
        .await?
        .json()
        .await?;
    if let Some(err) = resp.get("error") {
        bail!("eth_getTransactionReceipt error from {rpc_url}: {err}");
    }
    let receipt = match resp.get("result") {
        None | Some(serde_json::Value::Null) => return Ok(None),
        Some(receipt) => receipt,
    };

    let status = receipt.get("status").and_then(serde_json::Value::as_str);
    if status != Some("0x1") {
        bail!("settlement tx {tx_hash} did not succeed (receipt status {status:?})");
    }

    // topics = [Transfer, from, to]; `to` is the 32-byte left-padded recipient.
    let mut recipient_topic = [0u8; 32];
    recipient_topic[12..].copy_from_slice(recipient.as_slice());
    let recipient_topic = format!("0x{}", hex::encode(recipient_topic));

    let mut credited: u128 = 0;
    let logs = receipt
        .get("logs")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for log in logs {
        let address = log.get("address").and_then(serde_json::Value::as_str);
        if !address.is_some_and(|a| a.eq_ignore_ascii_case(&token.to_string())) {
            continue;
        }
        let topics = log
            .get("topics")
            .and_then(serde_json::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let [event, _from, to, ..] = topics else {
            continue;
        };
        let matches = event
            .as_str()
            .is_some_and(|t| t.eq_ignore_ascii_case(ERC20_TRANSFER_TOPIC))
            && to
                .as_str()
                .is_some_and(|t| t.eq_ignore_ascii_case(&recipient_topic));
        if !matches {
            continue;
        }
        let data = log
            .get("data")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("0x");
        let value = U256::from_str_radix(data.trim_start_matches("0x"), 16)
            .map_err(|e| anyhow!("transfer log in {tx_hash} has bad value {data}: {e}"))?;
        credited = credited.saturating_add(
            u128::try_from(value).map_err(|_| anyhow!("transfer value exceeds u128: {value}"))?,
        );
    }
    Ok(Some(credited))
}

/// Poll [`evm_erc20_credited_by_tx`] until the node has indexed `tx_hash`,
/// returning what it credited `recipient`. A receipt that is mined but failed
/// is fatal immediately; only "not indexed yet" and transient RPC errors are
/// retried.
pub async fn wait_for_evm_settlement_tx(
    rpc_url: &str,
    tx_hash: &str,
    token_contract: &str,
    recipient: &str,
    timeout_secs: u64,
) -> Result<u128> {
    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_secs);
    loop {
        match evm_erc20_credited_by_tx(rpc_url, tx_hash, token_contract, recipient).await {
            Ok(Some(credited)) => return Ok(credited),
            Ok(None) => debug!("settlement tx {tx_hash} not indexed yet; waiting..."),
            Err(e) if e.to_string().contains("did not succeed") => return Err(e),
            Err(e) => debug!("receipt read for {tx_hash} failed (retrying): {e:#}"),
        }
        if start.elapsed() >= timeout {
            bail!("timeout after {timeout_secs}s waiting for settlement tx {tx_hash} on {rpc_url}");
        }
        tokio::time::sleep(EVM_POLL_INTERVAL).await;
    }
}

/// Log where standing test funds live so a human can recover them later (the
/// derivation path in the module docs plus this address + balance is everything
/// needed to import the wallet and sweep).
pub async fn log_evm_recovery_balance(
    rpc_url: &str,
    token_contract: &str,
    holder: &str,
    asset: &str,
) {
    match evm_erc20_balance(rpc_url, token_contract, holder).await {
        Ok(balance) => info!(
            "[cross-chain-recovery] {asset} balance {balance} (base units) at {holder} \
             (token {token_contract}); recover via m/44'/60'/0'/0/0 of the test mnemonic"
        ),
        Err(e) => info!("[cross-chain-recovery] could not read {asset} balance at {holder}: {e:#}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evm_recipient_matches_known_vector() {
        // Hardhat/Anvil default mnemonic; account 0 at m/44'/60'/0'/0/0 is a
        // widely-published vector, so this pins our derivation to the standard
        // path without any network access.
        let mnemonic = "test test test test test test test test test test test junk";
        let (address, _signer) =
            mainnet_evm_recipient(mnemonic).expect("derivation should succeed");
        assert_eq!(
            address.to_lowercase(),
            "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"
        );
    }
}
