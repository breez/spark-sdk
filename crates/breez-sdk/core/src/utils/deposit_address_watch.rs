use std::collections::{HashMap, HashSet};

use platform_utils::time::SystemTime;

use crate::{
    chain::Utxo,
    persist::{UpdateWatchedAddressPayload, WatchedDepositAddress},
    utils::deposit_chain_syncer::TxOutput,
};

/// How long an address stays watched after being handed out.
const WATCH_TTL_SECS: u64 = 24 * 60 * 60;

/// What one pass found on a watched address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddressObservation {
    /// Nothing unconfirmed on it. Anything sent has confirmed or been spent.
    NothingToClaim,
    /// At least one unconfirmed deposit, which the watch keeps visible until the
    /// operators report it at a confirmation.
    Claimable,
}

/// What to do with each watched address this pass, decided before any polling.
///
/// `poll` and `retire` are exact complements: every watched address lands in
/// one of them. That is what keeps the poll set and the retirement rules from
/// disagreeing about which addresses are still interesting.
pub(crate) struct WatchPlan {
    /// Addresses to poll, newest first.
    pub poll: Vec<String>,
    /// Addresses not worth polling: their window closed and nothing ever
    /// arrived. Paired with the `issued_at` the decision was taken against.
    pub retire: Vec<(String, u64)>,
}

/// Sorts the watched addresses into the ones worth polling and the ones to drop.
///
/// An address that has taken a deposit is polled past its window, because the
/// window bounds waiting for a payment rather than following one that arrived:
/// dropping it would take a still-unconfirmed deposit out of
/// `list_unclaimed_deposits` until the operators report it. Handing an address
/// out again restarts its window.
pub(crate) fn plan_watch(watched: &[WatchedDepositAddress], now: u64) -> WatchPlan {
    let mut sorted: Vec<&WatchedDepositAddress> = watched.iter().collect();
    sorted.sort_by_key(|entry| std::cmp::Reverse(entry.issued_at));

    let mut plan = WatchPlan {
        poll: Vec::new(),
        retire: Vec::new(),
    };
    for entry in sorted {
        if entry.seen || !expired(entry, now) {
            plan.poll.push(entry.address.clone());
        } else {
            plan.retire.push((entry.address.clone(), entry.issued_at));
        }
    }
    plan
}

/// What to write back for the addresses this pass actually read: `Seen` for one
/// a deposit has just turned up on, `Unwatch` for one there is no longer any
/// reason to poll.
///
/// Only addresses carrying an observation are considered. An address the pass
/// did not get to, because a read failed, says nothing about whether its
/// deposits are still pending, so it is left exactly as it was.
pub(crate) fn observed_actions(
    watched_addresses: &[WatchedDepositAddress],
    observations: &HashMap<String, AddressObservation>,
    now: u64,
) -> Vec<(String, UpdateWatchedAddressPayload)> {
    let newest = watched_addresses.iter().max_by_key(|entry| entry.issued_at);
    let mut actions = Vec::new();
    for watched_address in watched_addresses {
        let Some(observation) = observations.get(&watched_address.address) else {
            continue;
        };
        let is_newest = newest.is_some_and(|n| n.address == watched_address.address);
        let unwatch = || UpdateWatchedAddressPayload::Unwatch {
            issued_at: watched_address.issued_at,
        };
        let action = match observation {
            AddressObservation::Claimable => {
                (!watched_address.seen).then_some(UpdateWatchedAddressPayload::Seen)
            }
            AddressObservation::NothingToClaim if watched_address.seen && !is_newest => {
                Some(unwatch())
            }
            AddressObservation::NothingToClaim => expired(watched_address, now).then(unwatch),
        };
        if let Some(payload) = action {
            actions.push((watched_address.address.clone(), payload));
        }
    }
    actions
}

/// The outputs the operator feed did not already report this pass.
pub(crate) fn unreported_utxos(utxos: Vec<Utxo>, already_seen: &HashSet<TxOutput>) -> Vec<Utxo> {
    utxos
        .into_iter()
        .filter(|utxo| {
            !already_seen.contains(&TxOutput {
                txid: utxo.txid.clone(),
                vout: utxo.vout,
            })
        })
        .collect()
}

/// Seconds since the epoch, or `None` when the system clock is unusable.
pub(crate) fn now_secs() -> Option<u64> {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

fn expired(entry: &WatchedDepositAddress, now: u64) -> bool {
    now.saturating_sub(entry.issued_at) > WATCH_TTL_SECS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::TxStatus;
    use macros::test_all;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    const DAY: u64 = 24 * 60 * 60;

    fn entry(address: &str, issued_at: u64, seen: bool) -> WatchedDepositAddress {
        WatchedDepositAddress {
            address: address.to_string(),
            issued_at,
            seen,
        }
    }

    fn utxo(txid: &str, vout: u32, confirmed: bool) -> Utxo {
        Utxo {
            txid: txid.to_string(),
            vout,
            value: 50_000,
            status: TxStatus {
                confirmed,
                block_height: confirmed.then_some(100),
                block_time: None,
            },
        }
    }

    // ============ plan_watch ============

    #[test_all]
    fn polls_newest_first_and_retires_the_expired() {
        let watched = vec![
            entry("old", 0, false),
            entry("current", 3 * DAY, false),
            entry("recent", 3 * DAY - 100, false),
        ];
        let plan = plan_watch(&watched, 3 * DAY);
        assert_eq!(plan.poll, vec!["current".to_string(), "recent".to_string()]);
        assert_eq!(plan.retire, vec![("old".to_string(), 0)]);
    }

    #[test_all]
    fn every_address_is_either_polled_or_retired() {
        // The property the split exists for: no address falls through both, and
        // none lands in both.
        let watched = vec![
            entry("a", 0, false),
            entry("b", 0, true),
            entry("c", 10 * DAY, false),
            entry("d", 10 * DAY, true),
        ];
        let plan = plan_watch(&watched, 10 * DAY);
        let mut covered: Vec<String> = plan.poll.clone();
        covered.extend(plan.retire.iter().map(|(a, _)| a.clone()));
        covered.sort();
        assert_eq!(covered, vec!["a", "b", "c", "d"]);
    }

    #[test_all]
    fn an_expired_address_is_retired_even_when_it_is_the_only_one() {
        // Nobody has asked for it in a day, so polling it is pure cost.
        let watched = vec![entry("current", 0, false)];
        let plan = plan_watch(&watched, 10 * DAY);
        assert!(plan.poll.is_empty());
        assert_eq!(plan.retire, vec![("current".to_string(), 0)]);
    }

    #[test_all]
    fn handing_an_address_out_again_restarts_its_window() {
        let stale = vec![entry("current", 0, false)];
        assert!(plan_watch(&stale, 10 * DAY).poll.is_empty());
        let reissued = vec![entry("current", 10 * DAY, false)];
        assert_eq!(
            plan_watch(&reissued, 10 * DAY).poll,
            vec!["current".to_string()]
        );
    }

    #[test_all]
    fn an_expired_address_that_took_a_deposit_is_still_polled() {
        // Its deposit may still be unconfirmed, and only a poll can tell.
        let watched = vec![entry("paid", 0, true), entry("unpaid", 0, false)];
        let plan = plan_watch(&watched, 10 * DAY);
        assert_eq!(plan.poll, vec!["paid".to_string()]);
        assert_eq!(plan.retire, vec![("unpaid".to_string(), 0)]);
    }

    // ============ observed_actions ============

    fn observed(pairs: &[(&str, AddressObservation)]) -> HashMap<String, AddressObservation> {
        pairs.iter().map(|(a, o)| ((*a).to_string(), *o)).collect()
    }

    #[test_all]
    fn a_first_sighting_marks_the_address_seen() {
        let watched = vec![entry("current", 100, false), entry("old", 50, false)];
        let actions = observed_actions(
            &watched,
            &observed(&[("old", AddressObservation::Claimable)]),
            100,
        );
        assert_eq!(
            actions,
            vec![("old".to_string(), UpdateWatchedAddressPayload::Seen)]
        );
    }

    #[test_all]
    fn an_already_seen_address_is_not_remarked() {
        let watched = vec![entry("current", 100, false), entry("old", 50, true)];
        let actions = observed_actions(
            &watched,
            &observed(&[("old", AddressObservation::Claimable)]),
            100,
        );
        assert!(actions.is_empty());
    }

    #[test_all]
    fn a_seen_address_that_goes_clear_retires() {
        let watched = vec![entry("current", 100, false), entry("old", 50, true)];
        let actions = observed_actions(
            &watched,
            &observed(&[("old", AddressObservation::NothingToClaim)]),
            100,
        );
        assert_eq!(
            actions,
            vec![(
                "old".to_string(),
                UpdateWatchedAddressPayload::Unwatch { issued_at: 50 }
            )]
        );
    }

    #[test_all]
    fn an_unseen_address_that_is_clear_keeps_waiting() {
        // Nothing has arrived yet, so only the window decides.
        let watched = vec![entry("current", 100, false), entry("old", 50, false)];
        let actions = observed_actions(
            &watched,
            &observed(&[("old", AddressObservation::NothingToClaim)]),
            100,
        );
        assert!(actions.is_empty());
    }

    #[test_all]
    fn an_address_the_pass_did_not_read_is_left_alone() {
        // A failed read says nothing about whether its deposits are pending, and
        // this is what stops the window retiring it on that silence.
        let watched = vec![entry("current", 100, false), entry("old", 0, true)];
        assert!(observed_actions(&watched, &HashMap::new(), 10 * DAY).is_empty());
    }

    #[test_all]
    fn the_newest_address_is_never_retired_by_deposit_state() {
        // It can be paid again at any time, so what its last deposit did decides
        // nothing.
        let watched = vec![entry("current", 100, true)];
        for observation in [
            AddressObservation::NothingToClaim,
            AddressObservation::Claimable,
        ] {
            let actions = observed_actions(&watched, &observed(&[("current", observation)]), 100);
            assert!(actions.is_empty(), "retired the current address");
        }
    }

    #[test_all]
    fn the_newest_address_still_expires() {
        // Otherwise a wallet that ever showed an address polls it forever.
        let watched = vec![entry("current", 0, true)];
        let actions = observed_actions(
            &watched,
            &observed(&[("current", AddressObservation::NothingToClaim)]),
            10 * DAY,
        );
        assert_eq!(
            actions,
            vec![(
                "current".to_string(),
                UpdateWatchedAddressPayload::Unwatch { issued_at: 0 }
            )]
        );
    }

    #[test_all]
    fn an_expired_address_holding_a_live_deposit_is_kept() {
        // The window bounds waiting for a payment, not claiming one that arrived.
        let watched = vec![entry("current", 0, true)];
        let actions = observed_actions(
            &watched,
            &observed(&[("current", AddressObservation::Claimable)]),
            10 * DAY,
        );
        assert!(actions.is_empty());
    }

    // ============ unreported_utxos ============

    #[test_all]
    fn drops_the_outputs_the_operators_already_reported() {
        let already_seen: HashSet<TxOutput> = [TxOutput {
            txid: "seen".to_string(),
            vout: 0,
        }]
        .into_iter()
        .collect();
        let kept = unreported_utxos(
            vec![utxo("seen", 0, false), utxo("fresh", 2, false)],
            &already_seen,
        );
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].txid, "fresh");
    }
}
