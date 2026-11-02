//! Shared `exact` on `lnbtc` vectors, fixture version 1, vendored from
//! x402-foundation/x402 (Apache-2.0) pull request 3727 at 63d5640d.
//!
//! The `http_server_adapter` cases are not run: they describe a server that
//! takes its public authority from `Host`, `:authority` or `X-Forwarded-Host`,
//! and the SDK's servers bind their configured origin without reading those.

use bitcoin::hashes::{Hash, sha256};
use serde_json::Value;

use super::{
    binding::{self, HttpRequest, McpToolCall},
    client::{
        IntendedRequest, PayerResult, PayerStatus, PreparedPayment, complete_payment,
        prepare_paid_payments, prepare_payment,
    },
    error::X402Result,
    network::LnbtcNetwork,
    types::{PaymentPayload, PaymentRequired},
};

mod settle;

const VECTORS: &str = include_str!("vectors/exact_lnbtc.json");
const FIXTURE_VERSION: u64 = 1;

pub(super) fn vectors() -> Value {
    let vectors: Value = serde_json::from_str(VECTORS).expect("vectors parse");
    assert_eq!(vectors["fixture_version"], FIXTURE_VERSION);
    vectors
}

fn str_vec(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("array")
        .iter()
        .map(|v| v.as_str().expect("string").to_string())
        .collect()
}

fn header_lines(value: &Value) -> Vec<(String, String)> {
    value
        .as_array()
        .expect("array")
        .iter()
        .map(|line| {
            (
                line[0].as_str().expect("name").to_string(),
                line[1].as_str().expect("value").to_string(),
            )
        })
        .collect()
}

/// The outcomes a case allows: one expectation, or any of an open question's.
fn allowed_outcomes(expect: &Value) -> Vec<&Value> {
    match expect.get("outcomes") {
        Some(outcomes) => outcomes.as_object().expect("outcomes").values().collect(),
        None => vec![expect],
    }
}

fn check_binding(id: &str, expect: &Value, description: &X402Result<String>) {
    let matched = allowed_outcomes(expect).into_iter().any(|outcome| {
        match (description, outcome.get("request_hash")) {
            (Ok(description), Some(hash)) => {
                let digest =
                    hex::encode(sha256::Hash::hash(description.as_bytes()).to_byte_array());
                outcome
                    .get("canonical_description")
                    .is_none_or(|c| c == description.as_str())
                    && hash == digest.as_str()
            }
            (Err(e), None) => outcome["error"] == e.reason.as_str(),
            _ => false,
        }
    });
    assert!(matched, "{id}: got {description:?}, expected {expect}");
}

#[macros::test_all]
fn http_binding_vectors() {
    let vectors = vectors();
    for case in vectors["http_binding"].as_array().expect("cases") {
        let input = &case["input"];
        let body = hex::decode(input["body_hex"].as_str().expect("body")).expect("hex");
        let headers = header_lines(&input["header_lines"]);
        let request = HttpRequest {
            method: input["method"].as_str().expect("method"),
            url: input["url"].as_str().expect("url"),
            body: &body,
            headers: &headers,
        };
        let description = binding::http_description(&request, &str_vec(&input["bound_headers"]));
        check_binding(
            case["id"].as_str().expect("id"),
            &case["expect"],
            &description,
        );
    }
}

/// Parses the case's params as the SDK parses a caller's JSON. A refusal
/// counts as a binding rejection.
fn mcp_params(input: &Value) -> X402Result<Value> {
    match input.get("params_json") {
        Some(text) => binding::parse_tool_call_params(text.as_str().expect("params_json")),
        None => Ok(input["params"].clone()),
    }
}

#[macros::test_all]
fn mcp_binding_vectors() {
    let vectors = vectors();
    for case in vectors["mcp_binding"].as_array().expect("cases") {
        let input = &case["input"];
        let description = mcp_params(input).and_then(|params| {
            let call = McpToolCall {
                server: input["server"].as_str().expect("server"),
                params: &params,
            };
            binding::mcp_description(&call, &str_vec(&input["bound_metadata"]))
        });
        check_binding(
            case["id"].as_str().expect("id"),
            &case["expect"],
            &description,
        );
    }
}

/// Applies an RFC 6902 patch limited to `add`, `remove` and `replace` on
/// object members, which is all the vectors use.
pub(super) fn apply_patch(doc: &mut Value, patch: &Value) {
    for op in patch.as_array().expect("patch") {
        let pointer = op["path"].as_str().expect("path");
        let (parent, key) = pointer.rsplit_once('/').expect("member path");
        let key = key.replace("~1", "/").replace("~0", "~");
        let target = doc
            .pointer_mut(parent)
            .and_then(Value::as_object_mut)
            .unwrap_or_else(|| panic!("patch target {parent} is not an object"));
        match op["op"].as_str().expect("op") {
            "add" | "replace" => {
                target.insert(key, op["value"].clone());
            }
            "remove" => {
                target.remove(&key);
            }
            other => panic!("unsupported patch op {other}"),
        }
    }
}

enum ClientOutcome {
    Rejected {
        reason: String,
        detail: String,
        payer_called: bool,
    },
    Paid(PaymentPayload),
}

/// Runs a client case at `now` and `skew`, or as an invoice already paid when
/// `clock` is unset.
fn run_client_case(doc: &Value, clock: Option<(u64, u64)>) -> ClientOutcome {
    let payment_required: PaymentRequired =
        serde_json::from_value(doc["payment_required"].clone()).expect("payment_required");
    let intended = &doc["intended_request"];
    let body;
    let headers;
    let params;
    let request = match intended["profile"].as_str().expect("profile") {
        "http:1" => {
            let http = &intended["http"];
            body = hex::decode(http["body_hex"].as_str().expect("body")).expect("hex");
            headers = header_lines(&http["header_lines"]);
            IntendedRequest::Http(HttpRequest {
                method: http["method"].as_str().expect("method"),
                url: http["url"].as_str().expect("url"),
                body: &body,
                headers: &headers,
            })
        }
        "mcp:1" => {
            let mcp = &intended["mcp"];
            params = mcp["params"].clone();
            IntendedRequest::Mcp(McpToolCall {
                server: mcp["server"].as_str().expect("server"),
                params: &params,
            })
        }
        other => panic!("unknown profile {other}"),
    };

    let prepared = match clock {
        Some((now, skew)) => prepare_payment(
            &payment_required,
            &request,
            LnbtcNetwork::Mainnet,
            now,
            skew,
        ),
        None => prepare_paid_payments(&payment_required, &request, LnbtcNetwork::Mainnet)
            .map(|mut paid| paid.remove(0)),
    };
    let prepared: PreparedPayment = match prepared {
        Ok(prepared) => prepared,
        Err(e) => {
            return ClientOutcome::Rejected {
                reason: e.reason.as_str().to_string(),
                detail: e.detail,
                payer_called: false,
            };
        }
    };

    let payer = &doc["payer_result"];
    let result = PayerResult {
        status: match payer["status"].as_str().expect("status") {
            "paid" => PayerStatus::Paid,
            "unpaid" => PayerStatus::Unpaid,
            "in_flight" => PayerStatus::InFlight,
            other => panic!("unknown payer status {other}"),
        },
        invoice: payer["invoice"].as_str().expect("invoice").to_string(),
        payment_hash: payer["payment_hash"].as_str().expect("hash").to_string(),
        amount_msat: payer["amount_msat"]
            .as_str()
            .expect("amount")
            .parse()
            .expect("amount"),
        preimage: payer["preimage"].as_str().map(str::to_string),
    };
    match complete_payment(&prepared, &result) {
        Ok(payload) => ClientOutcome::Paid(payload),
        Err(e) => ClientOutcome::Rejected {
            reason: e.reason.as_str().to_string(),
            detail: e.detail,
            payer_called: true,
        },
    }
}

#[macros::test_all]
fn client_vectors() {
    let vectors = vectors();
    let client = &vectors["client"];
    for case in client["cases"].as_array().expect("cases") {
        let id = case["id"].as_str().expect("id");
        let mut doc = client["base"].clone();
        apply_patch(&mut doc, &case["patch"]);
        let now = case["now"].as_u64().expect("now");
        let skew = case["skew"].as_u64().expect("skew");
        let outcome = run_client_case(&doc, Some((now, skew)));
        let matched = allowed_outcomes(&case["expect"])
            .into_iter()
            .any(|expect| match &outcome {
                ClientOutcome::Paid(payload) => {
                    expect
                        .get("preimage")
                        .is_some_and(|p| *p == payload.payload["preimage"])
                        && payload.accepted == doc["payment_required"]["accepts"][0]
                }
                ClientOutcome::Rejected {
                    reason,
                    payer_called,
                    ..
                } => {
                    expect.get("error").is_some_and(|e| e == reason.as_str())
                        && expect
                            .get("payer_called")
                            .is_none_or(|c| c.as_bool() == Some(*payer_called))
                }
            });
        let got = match &outcome {
            ClientOutcome::Paid(_) => "paid".to_string(),
            ClientOutcome::Rejected {
                reason,
                detail,
                payer_called,
            } => format!("{reason}: {detail} (payer called: {payer_called})"),
        };
        assert!(matched, "{id}: got {got}, expected {}", case["expect"]);
    }
}

#[macros::test_all]
fn paid_invoice_skips_time_checks() {
    let vectors = vectors();
    let client = &vectors["client"];
    for id in ["invoice-expired", "invoice-created-in-future"] {
        let case = client["cases"]
            .as_array()
            .expect("cases")
            .iter()
            .find(|case| case["id"] == id)
            .expect("case");
        let mut doc = client["base"].clone();
        apply_patch(&mut doc, &case["patch"]);
        let ClientOutcome::Paid(payload) = run_client_case(&doc, None) else {
            panic!("{id}: paid invoice was refused");
        };
        assert_eq!(payload.payload["preimage"], doc["payer_result"]["preimage"]);
    }
}

#[macros::test_all]
fn paid_entries_include_every_entry_but_payment_picks_a_payable_one() {
    let vectors = vectors();
    let client = &vectors["client"];
    let expired_case = client["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .find(|case| case["id"] == "invoice-expired")
        .expect("case");
    let expired_entry = expired_case["patch"][0]["value"][0].clone();
    let mut doc = client["base"].clone();
    let valid_entry = doc["payment_required"]["accepts"][0].clone();
    doc["payment_required"]["accepts"] = serde_json::json!([expired_entry, valid_entry]);
    let payment_required: PaymentRequired =
        serde_json::from_value(doc["payment_required"].clone()).expect("payment_required");
    let http = &doc["intended_request"]["http"];
    let body = hex::decode(http["body_hex"].as_str().expect("body")).expect("hex");
    let headers = header_lines(&http["header_lines"]);
    let request = IntendedRequest::Http(HttpRequest {
        method: http["method"].as_str().expect("method"),
        url: http["url"].as_str().expect("url"),
        body: &body,
        headers: &headers,
    });
    let now = expired_case["now"].as_u64().expect("now");

    let paid = prepare_paid_payments(&payment_required, &request, LnbtcNetwork::Mainnet).unwrap();
    let payable =
        prepare_payment(&payment_required, &request, LnbtcNetwork::Mainnet, now, 60).unwrap();
    assert_eq!(paid.len(), 2);
    assert_eq!(paid[0].accepted, expired_entry);
    assert_eq!(payable.accepted, valid_entry);
}

#[macros::test_all]
fn payment_required_header_decodes_with_or_without_padding() {
    use base64::{Engine, prelude::BASE64_STANDARD_NO_PAD};

    let vectors = vectors();
    let payment_required = &vectors["client"]["base"]["payment_required"];
    let json = serde_json::to_vec(payment_required).expect("serializes");
    for header in [
        base64::prelude::BASE64_STANDARD.encode(&json),
        BASE64_STANDARD_NO_PAD.encode(&json),
    ] {
        let decoded = PaymentRequired::from_header(&header).expect("decodes");
        assert_eq!(decoded.accepts[0], payment_required["accepts"][0]);
        assert_eq!(decoded.resource.url, "https://api.example.com/article/A");
    }
    assert!(PaymentRequired::from_header("not base64!").is_err());
}

#[macros::test_all]
fn payment_required_parses_from_mcp_json() {
    let vectors = vectors();
    let payment_required = &vectors["client"]["base"]["payment_required"];
    let decoded =
        PaymentRequired::from_header_or_json(&payment_required.to_string()).expect("parses JSON");
    assert_eq!(decoded.accepts[0], payment_required["accepts"][0]);
}
