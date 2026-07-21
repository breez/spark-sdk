//! EVM-side helpers for the mainnet cross-chain itests. Two roles:
//!
//! - **Send verification**: derive the deterministic recipient from the test
//!   mnemonic and read ERC-20 balances via JSON-RPC to confirm a cross-chain
//!   send actually landed, independent of the SDK's own status reporting.
//! - **Receive broadcast**: sign and submit an EIP-1559 ERC-20 transfer from
//!   the same deterministic key into the deposit address returned by the SDK's
//!   cross-chain receive flow, then wait for the transaction receipt.
//!
//! The recipient is derived at the standard Ethereum path `m/44'/60'/0'/0/0`
//! with an empty BIP-39 passphrase, from the same mnemonic the test account
//! ("Alice") uses. Importing that mnemonic into any EVM wallet at the default
//! path reproduces this address and its private key, so any funds accumulated
//! there are recoverable off-band.

use std::time::{Duration, Instant};

use alloy_consensus::{SignableTransaction, TxEip1559, TxEnvelope};
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::{Address, Bytes, TxKind, U256};
use alloy_signer::SignerSync;
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

/// ERC-20 `transfer(address,uint256)` selector.
const ERC20_TRANSFER_SELECTOR: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];

/// Poll interval for external-chain reads (balance change, tx receipt). Kept
/// gentle so public RPC endpoints don't rate-limit during long waits.
const EVM_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Shared HTTP client, built once. `wait_for_evm_balance_increase` reads the
/// balance every few seconds over a multi-minute window, so a fresh client (and
/// its connection pool / TLS setup) per call would be wasteful.
fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// Derive the deterministic EVM recipient (checksummed address string + signer)
/// from the mainnet test mnemonic. See the module docs for the path / recovery.
///
/// The signer is returned alongside the address so the receive tests can sign
/// outbound ERC-20 transfers with the same key that owns the deposit.
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

/// Read the holder's native-token (ETH) balance in wei.
pub async fn evm_native_balance(rpc_url: &str, holder: &str) -> Result<u128> {
    let holder: Address = holder
        .parse()
        .map_err(|e| anyhow!("invalid holder address {holder}: {e}"))?;
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_getBalance",
        "params": [holder.to_string(), "latest"],
    });
    let resp: serde_json::Value = http_client()
        .post(rpc_url)
        .json(&body)
        .send()
        .await?
        .json()
        .await?;
    if let Some(err) = resp.get("error") {
        bail!("eth_getBalance error from {rpc_url}: {err}");
    }
    let result = resp
        .get("result")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("eth_getBalance returned no result: {resp}"))?;
    let value = U256::from_str_radix(result.trim_start_matches("0x"), 16)
        .map_err(|e| anyhow!("eth_getBalance: bad hex result {result}: {e}"))?;
    u128::try_from(value).map_err(|_| anyhow!("native balance exceeds u128: {value}"))
}

/// Sign and broadcast an ERC-20 `transfer(to, amount)` as an EIP-1559 tx.
/// Fetches chain id / nonce / gas price / gas estimate via JSON-RPC, signs
/// with the caller-supplied key, and submits via `eth_sendRawTransaction`.
/// Returns the transaction hash.
pub async fn evm_send_erc20(
    rpc_url: &str,
    signer: &PrivateKeySigner,
    token_contract: &str,
    to: &str,
    amount: u128,
) -> Result<String> {
    let token: Address = token_contract
        .parse()
        .map_err(|e| anyhow!("invalid token contract {token_contract}: {e}"))?;
    let to_addr: Address = to
        .parse()
        .map_err(|e| anyhow!("invalid recipient address {to}: {e}"))?;

    let mut calldata = Vec::with_capacity(4 + 32 + 32);
    calldata.extend_from_slice(&ERC20_TRANSFER_SELECTOR);
    calldata.extend_from_slice(&[0u8; 12]);
    calldata.extend_from_slice(to_addr.as_slice());
    let amount_bytes: [u8; 32] = U256::from(amount).to_be_bytes();
    calldata.extend_from_slice(&amount_bytes);
    let calldata = Bytes::from(calldata);

    let from = signer.address();
    let chain_id = rpc_hex_u64(rpc_url, "eth_chainId", json!([])).await?;
    let nonce = rpc_hex_u64(
        rpc_url,
        "eth_getTransactionCount",
        json!([from.to_string(), "pending"]),
    )
    .await?;
    let gas_price = rpc_hex_u128(rpc_url, "eth_gasPrice", json!([])).await?;
    // No priority tip: tests should minimize gas cost, and the Arbitrum
    // sequencer accepts a zero tip. Buffer the base-fee ceiling by 2x so a
    // small mid-flight bump doesn't reject the tx.
    let max_fee_per_gas = gas_price.saturating_mul(2);

    // Gas estimate for the calldata; the RPC accepts an object with `from`,
    // `to`, and `data`. Overshoot slightly (25%) to cover the L1 calldata
    // portion on Arbitrum.
    let est_body = json!([{
        "from": from.to_string(),
        "to": token.to_string(),
        "data": format!("0x{}", hex::encode(&calldata)),
    }]);
    let est_gas = rpc_hex_u128(rpc_url, "eth_estimateGas", est_body).await?;
    let gas_limit = est_gas.saturating_mul(125) / 100;
    let gas_limit_u64 =
        u64::try_from(gas_limit).map_err(|_| anyhow!("estimated gas {est_gas} exceeds u64"))?;

    let tx = TxEip1559 {
        chain_id,
        nonce,
        gas_limit: gas_limit_u64,
        max_fee_per_gas,
        max_priority_fee_per_gas: 0,
        to: TxKind::Call(token),
        value: U256::ZERO,
        access_list: Default::default(),
        input: calldata,
    };
    let signature = signer
        .sign_hash_sync(&tx.signature_hash())
        .map_err(|e| anyhow!("sign tx: {e}"))?;
    let envelope: TxEnvelope = tx.into_signed(signature).into();
    let raw = envelope.encoded_2718();
    let raw_hex = format!("0x{}", hex::encode(&raw));

    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "eth_sendRawTransaction",
        "params": [raw_hex],
    });
    let resp: serde_json::Value = http_client()
        .post(rpc_url)
        .json(&body)
        .send()
        .await?
        .json()
        .await?;
    if let Some(err) = resp.get("error") {
        bail!("eth_sendRawTransaction error from {rpc_url}: {err}");
    }
    let tx_hash = resp
        .get("result")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("eth_sendRawTransaction returned no result: {resp}"))?
        .to_string();
    info!(
        "Broadcast ERC-20 transfer: from={from} to={to_addr} token={token} \
         amount={amount} tx={tx_hash}"
    );
    Ok(tx_hash)
}

/// Poll `eth_getTransactionReceipt` until the tx lands and returns `status: 0x1`
/// (success). Bails if the tx reverts or the timeout elapses.
pub async fn wait_for_evm_tx_confirmation(
    rpc_url: &str,
    tx_hash: &str,
    timeout_secs: u64,
) -> Result<()> {
    let start = Instant::now();
    loop {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_getTransactionReceipt",
            "params": [tx_hash],
        });
        match http_client().post(rpc_url).json(&body).send().await {
            Ok(r) => match r.json::<serde_json::Value>().await {
                Ok(resp) => {
                    if let Some(receipt) = resp.get("result").filter(|v| !v.is_null()) {
                        let status = receipt
                            .get("status")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("");
                        if status == "0x1" {
                            return Ok(());
                        }
                        bail!("tx {tx_hash} reverted (status {status}): {receipt}");
                    }
                    debug!("tx {tx_hash} not yet mined; waiting...");
                }
                Err(e) => debug!("receipt decode failed (retrying): {e:#}"),
            },
            Err(e) => debug!("receipt request failed (retrying): {e:#}"),
        }
        if start.elapsed() >= Duration::from_secs(timeout_secs) {
            bail!("timeout after {timeout_secs}s waiting for tx {tx_hash} to confirm");
        }
        tokio::time::sleep(EVM_POLL_INTERVAL).await;
    }
}

/// Shared JSON-RPC helper: POST `{method, params}`, decode the `0x…` hex
/// `result` as a `u64`. For scalar-return methods (`eth_chainId`, nonce, gas).
async fn rpc_hex_u64(rpc_url: &str, method: &str, params: serde_json::Value) -> Result<u64> {
    let value = rpc_hex_u128(rpc_url, method, params).await?;
    u64::try_from(value).map_err(|_| anyhow!("{method} result exceeds u64: {value}"))
}

async fn rpc_hex_u128(rpc_url: &str, method: &str, params: serde_json::Value) -> Result<u128> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let resp: serde_json::Value = http_client()
        .post(rpc_url)
        .json(&body)
        .send()
        .await?
        .json()
        .await?;
    if let Some(err) = resp.get("error") {
        bail!("{method} error from {rpc_url}: {err}");
    }
    let result = resp
        .get("result")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("{method} returned no result: {resp}"))?;
    let value = U256::from_str_radix(result.trim_start_matches("0x"), 16)
        .map_err(|e| anyhow!("{method}: bad hex result {result}: {e}"))?;
    u128::try_from(value).map_err(|_| anyhow!("{method} result exceeds u128: {value}"))
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
