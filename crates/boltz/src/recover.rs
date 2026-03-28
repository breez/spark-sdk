use std::collections::{HashMap, HashSet};

use alloy_primitives::{Address, U256};

use crate::config::{RECOVERY_MAX_KEY_INDEX, RECOVERY_SCAN_BATCH_SIZE};
use crate::error::BoltzError;
use crate::evm::contracts::{
    DecodedLockupEvent, address_to_topic, decode_hash_values_return, decode_lockup_event,
    decode_swaps_check_return, encode_hash_values, encode_swaps_check, lockup_event_topic,
};
use crate::evm::provider::EvmProvider;
use crate::keys::EvmKeyManager;

/// A swap discovered on-chain that is still claimable.
#[derive(Debug, Clone)]
pub struct RecoverableSwap {
    /// The key derivation index that produced the matching preimage hash.
    pub key_index: u32,
    /// The preimage (derived from seed + index).
    pub preimage: [u8; 32],
    /// The preimage hash (from the on-chain Lockup event).
    pub preimage_hash: [u8; 32],
    /// tBTC amount locked (EVM units, 18 decimals).
    pub amount: U256,
    /// Token address locked in the swap.
    pub token_address: Address,
    /// Claim address / gas signer.
    pub claim_address: Address,
    /// Boltz refund address.
    pub refund_address: Address,
    /// Timelock block height.
    pub timelock: U256,
    /// Block number of the Lockup event.
    pub block_number: u64,
    /// Transaction hash of the Lockup event.
    pub lockup_tx_hash: String,
}

/// Statistics from a recovery scan.
#[derive(Debug, Clone, Default)]
pub struct ScanStats {
    /// Total Lockup events found matching our claim address.
    pub total_events: u32,
    /// Events where the swap was already claimed/refunded.
    pub already_settled: u32,
    /// Highest key index found among matched events.
    pub highest_key_index: Option<u32>,
}

/// Scan the `ERC20Swap` contract for Lockup events and identify recoverable swaps.
///
/// Flow (scan-first approach — avoids expensive key derivation when nothing to recover):
/// 1. Derive gas signer address
/// 2. Scan Lockup events filtered by our claim address
/// 3. If no events found, return immediately
/// 4. Collect preimage hashes from events
/// 5. Derive keys, matching against found hashes (stop early when all matched)
/// 6. Check on-chain if matched swaps are still locked
#[expect(clippy::too_many_lines)]
pub async fn scan_for_recoverable_swaps(
    evm_provider: &EvmProvider,
    key_manager: &EvmKeyManager,
    chain_id: u32,
    erc20swap_address: &str,
    deploy_block: u64,
) -> Result<(Vec<RecoverableSwap>, ScanStats), BoltzError> {
    let gas_signer = key_manager.derive_gas_signer(chain_id)?;
    let claim_topic = address_to_topic(&gas_signer.address);
    let event_topic = format!("0x{}", hex::encode(lockup_event_topic()));

    let current_block = evm_provider.eth_block_number().await?;
    tracing::info!(
        current_block,
        deploy_block,
        claim_address = hex::encode(gas_signer.address),
        "Scanning for Lockup events"
    );

    // Phase 1: Scan all Lockup events for our claim address
    let mut events = Vec::new();
    let mut to_block = current_block;

    while to_block > deploy_block {
        let from_block = to_block
            .saturating_sub(RECOVERY_SCAN_BATCH_SIZE)
            .max(deploy_block);

        let logs = evm_provider
            .eth_get_logs(
                erc20swap_address,
                &[
                    Some(&event_topic), // topic0: Lockup event signature
                    None,               // topic1: preimageHash (wildcard)
                    Some(&claim_topic), // topic2: claimAddress (our gas signer)
                ],
                from_block,
                to_block,
            )
            .await?;

        for log in &logs {
            match decode_lockup_event(log) {
                Ok(event) => events.push(event),
                Err(e) => tracing::warn!(error = %e, "Failed to decode Lockup event, skipping"),
            }
        }

        if from_block == deploy_block {
            break;
        }
        to_block = from_block.saturating_sub(1);
    }

    let mut stats = ScanStats {
        total_events: u32::try_from(events.len()).unwrap_or(u32::MAX),
        ..ScanStats::default()
    };

    if events.is_empty() {
        tracing::info!("No Lockup events found for this wallet");
        return Ok((Vec::new(), stats));
    }

    tracing::info!(
        count = events.len(),
        "Found Lockup events, deriving keys to match"
    );

    // Phase 2: Derive preimage hashes and match against found events
    let mut unmatched: HashSet<[u8; 32]> = events.iter().map(|e| e.preimage_hash).collect();
    let mut matched: HashMap<[u8; 32], (u32, [u8; 32])> = HashMap::new();

    for index in 0..RECOVERY_MAX_KEY_INDEX {
        if unmatched.is_empty() {
            break;
        }

        let preimage_hash = key_manager.derive_preimage_hash(chain_id, index)?;
        if unmatched.remove(&preimage_hash) {
            let preimage = key_manager.derive_preimage(chain_id, index)?;
            matched.insert(preimage_hash, (index, preimage));
            stats.highest_key_index = Some(stats.highest_key_index.map_or(index, |h| h.max(index)));
        }
    }

    if matched.is_empty() {
        tracing::info!("No events matched derived keys");
        return Ok((Vec::new(), stats));
    }

    tracing::info!(
        matched = matched.len(),
        "Matched events, checking on-chain state"
    );

    // Phase 3: Check on-chain state and build recoverable list
    let mut recoverable = Vec::new();

    for event in &events {
        let Some(&(index, preimage)) = matched.get(&event.preimage_hash) else {
            continue;
        };

        if is_swap_still_locked(evm_provider, erc20swap_address, event).await? {
            tracing::info!(
                key_index = index,
                tx = event.transaction_hash,
                block = event.block_number,
                "Found recoverable swap"
            );
            recoverable.push(RecoverableSwap {
                key_index: index,
                preimage,
                preimage_hash: event.preimage_hash,
                amount: event.amount,
                token_address: event.token_address,
                claim_address: event.claim_address,
                refund_address: event.refund_address,
                timelock: event.timelock,
                block_number: event.block_number,
                lockup_tx_hash: event.transaction_hash.clone(),
            });
        } else {
            stats.already_settled = stats.already_settled.saturating_add(1);
        }
    }

    tracing::info!(
        total_events = stats.total_events,
        already_settled = stats.already_settled,
        recoverable = recoverable.len(),
        "Recovery scan complete"
    );

    Ok((recoverable, stats))
}

/// Check whether a swap is still locked on-chain (not yet claimed/refunded).
async fn is_swap_still_locked(
    evm_provider: &EvmProvider,
    erc20swap_address: &str,
    event: &DecodedLockupEvent,
) -> Result<bool, BoltzError> {
    let hash_calldata = encode_hash_values(
        event.preimage_hash,
        event.amount,
        event.token_address,
        event.claim_address,
        event.refund_address,
        event.timelock,
    );
    let hash_result = evm_provider
        .eth_call(erc20swap_address, &hash_calldata)
        .await?;
    let swap_hash = decode_hash_values_return(&hash_result)?;

    let check_calldata = encode_swaps_check(swap_hash);
    let check_result = evm_provider
        .eth_call(erc20swap_address, &check_calldata)
        .await?;
    decode_swaps_check_return(&check_result)
}
