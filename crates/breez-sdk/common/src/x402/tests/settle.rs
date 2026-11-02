use std::collections::HashSet;

use serde_json::Value;

use super::{apply_patch, vectors};
use crate::x402::{server::verify_settlement, types::PaymentPayload};

/// What one settle attempt produced: the replay key and retention on
/// success, or the failure reason.
enum Outcome {
    Settled {
        transaction: String,
        network: String,
        replay_key: String,
        retain_until: u64,
    },
    Failed(String),
}

fn settle(doc: &Value, now: u64, skew: u64, store: &mut HashSet<String>) -> Outcome {
    let payload: PaymentPayload = match serde_json::from_value(doc["payload"].clone()) {
        Ok(payload) => payload,
        Err(_) => return Outcome::Failed("invalid_payload".to_string()),
    };
    match verify_settlement(&doc["requirements"], &payload, now, skew) {
        Err(e) => Outcome::Failed(e.reason.as_str().to_string()),
        Ok(verified) => {
            let key = verified.consumption_key();
            if !store.insert(key.clone()) {
                return Outcome::Failed("duplicate_settlement".to_string());
            }
            let response = verified.settlement_response();
            Outcome::Settled {
                transaction: response.transaction,
                network: response.network,
                replay_key: key,
                retain_until: verified.retain_until_secs(skew),
            }
        }
    }
}

#[macros::test_all]
fn facilitator_settle_vectors() {
    let vectors = vectors();
    let settle_vectors = &vectors["facilitator_settle"];
    let mut mismatches = Vec::new();
    for case in settle_vectors["cases"].as_array().expect("cases") {
        let id = case["id"].as_str().expect("id");
        let mut store = HashSet::new();
        for (i, step) in case["steps"].as_array().expect("steps").iter().enumerate() {
            let mut doc = settle_vectors["base"].clone();
            apply_patch(&mut doc, &step["patch"]);
            let now = step["now"].as_u64().expect("now");
            let skew = step["skew"].as_u64().expect("skew");
            let expect = &step["expect"];
            let matched = match settle(&doc, now, skew, &mut store) {
                Outcome::Settled {
                    transaction,
                    network,
                    replay_key,
                    retain_until,
                } => {
                    expect["success"] == true
                        && expect["transaction"] == transaction.as_str()
                        && expect["network"] == network.as_str()
                        && expect["replay_key"] == replay_key.as_str()
                        && expect["retain_until_at_least"]
                            .as_u64()
                            .is_some_and(|at_least| retain_until >= at_least)
                }
                Outcome::Failed(reason) => expect["error_reason"] == reason.as_str(),
            };
            if !matched {
                mismatches.push(format!("{id} step {i}: expected {expect}"));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[macros::test_all]
fn unsupported_version_is_refused() {
    let mut doc = vectors()["facilitator_settle"]["base"].clone();
    doc["payload"]["x402Version"] = 1.into();
    let outcome = settle(&doc, 1_700_000_000, 60, &mut HashSet::new());
    assert!(
        matches!(&outcome, Outcome::Failed(reason) if reason == "invalid_x402_version"),
        "unsupported version was settled"
    );
}
