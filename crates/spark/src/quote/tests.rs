use base64::Engine;
use bitcoin::secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
use macros::test_all;
use prost::Message as _;
use serde_json::Value;

use super::hashing::envelope_digest;
use super::*;
use crate::operator::rpc::spark::manifest_amount::Amount;
use crate::operator::rpc::spark::{
    FeeComponent, FeeRole, FeeSource, ManifestAmount, ManifestEdge, Network, TransferManifest,
};

const QUOTE_ENVELOPE_CASES: &str = include_str!("testdata/quote_envelope_cases.json");
const MANIFEST_HASH_CASES: &str = include_str!("testdata/transfer_manifest_hash_cases.json");
const RESOLUTION_CASES: &str = include_str!("testdata/transfer_manifest_resolution_cases.json");

fn cases(file: &str, key: &str) -> Vec<Value> {
    let parsed: Value = serde_json::from_str(file).unwrap();
    parsed[key].as_array().unwrap().clone()
}

fn hex_field(case: &Value, key: &str) -> Vec<u8> {
    hex::decode(case[key].as_str().unwrap_or_default()).unwrap()
}

fn hash32(case: &Value, key: &str) -> [u8; 32] {
    hex_field(case, key).try_into().unwrap()
}

fn name(case: &Value) -> &str {
    case["name"].as_str().unwrap()
}

// ─── manifest hash vectors ────────────────────────────────────────────────

/// Builds the proto from the vectors' canonical proto JSON. Enums may be names
/// or raw numbers, so unknown values reach the hasher as they would on the wire.
fn manifest_from_json(json: &Value) -> TransferManifest {
    fn bytes(value: &Value) -> Vec<u8> {
        value
            .as_str()
            .map(|b64| {
                base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .unwrap()
            })
            .unwrap_or_default()
    }
    fn enum_value(value: &Value, from_name: impl Fn(&str) -> Option<i32>) -> i32 {
        match value {
            Value::Null => 0,
            Value::String(name) => from_name(name).unwrap(),
            other => i32::try_from(other.as_i64().unwrap()).unwrap(),
        }
    }
    fn amount(value: &Value) -> Option<ManifestAmount> {
        let amount = if let Some(sats) = value.get("sats") {
            Amount::Sats(sats.as_str().unwrap().parse().unwrap())
        } else if let Some(bps) = value.get("bps") {
            Amount::Bps(u32::try_from(bps.as_u64().unwrap()).unwrap())
        } else {
            return None;
        };
        Some(ManifestAmount {
            amount: Some(amount),
        })
    }
    fn timestamp(value: &Value) -> Option<prost_types::Timestamp> {
        let parsed = chrono::DateTime::parse_from_rfc3339(value.as_str()?).unwrap();
        Some(prost_types::Timestamp {
            seconds: parsed.timestamp(),
            nanos: i32::try_from(parsed.timestamp_subsec_nanos()).unwrap(),
        })
    }
    let list = |key: &str| json[key].as_array().cloned().unwrap_or_default();

    TransferManifest {
        version: u32::try_from(json["version"].as_u64().unwrap_or_default()).unwrap(),
        transfer_id: json["transferId"].as_str().unwrap_or_default().to_string(),
        network: enum_value(&json["network"], |n| {
            Network::from_str_name(n).map(i32::from)
        }),
        transfer_expiry_time: timestamp(&json["transferExpiryTime"]),
        edges: list("edges")
            .iter()
            .map(|edge| ManifestEdge {
                sender_identity_public_key: bytes(&edge["senderIdentityPublicKey"]),
                receiver_identity_public_key: bytes(&edge["receiverIdentityPublicKey"]),
                amount: amount(&edge["amount"]),
            })
            .collect(),
        fees: list("fees")
            .iter()
            .map(|fee| FeeComponent {
                source: enum_value(&fee["source"], |n| {
                    FeeSource::from_str_name(n).map(i32::from)
                }),
                role: enum_value(&fee["role"], |n| FeeRole::from_str_name(n).map(i32::from)),
                amount: amount(&fee["amount"]),
                receiver_identity_public_key: bytes(&fee["receiverIdentityPublicKey"]),
            })
            .collect(),
        quote_expiry_time: timestamp(&json["quoteExpiryTime"]),
    }
}

#[test_all]
fn manifest_hash_matches_every_golden_vector() {
    let cases = cases(MANIFEST_HASH_CASES, "testCases");
    assert!(!cases.is_empty());
    for case in cases {
        let manifest = manifest_from_json(&case["transferManifest"]);
        let hash =
            transfer_manifest_hash(&manifest).unwrap_or_else(|e| panic!("{}: {e}", name(&case)));
        assert_eq!(
            hex::encode(hash),
            case["expectedHash"].as_str().unwrap(),
            "{}",
            name(&case)
        );
        assert_eq!(
            serialized_transfer_manifest_hash(&manifest.encode_to_vec()).unwrap(),
            hash
        );
    }
}

#[test_all]
fn manifest_hash_refuses_every_invalid_vector() {
    let cases = cases(MANIFEST_HASH_CASES, "invalidCases");
    assert!(!cases.is_empty());
    for case in cases {
        let manifest = manifest_from_json(&case["transferManifest"]);
        assert!(
            matches!(
                transfer_manifest_hash(&manifest),
                Err(QuoteError::NotHashable(_))
            ),
            "{} was hashed",
            name(&case)
        );
    }
}

// ─── quote envelope vectors ───────────────────────────────────────────────

#[test_all]
fn reason_and_role_numbering_matches_the_vectors() {
    let parsed: Value = serde_json::from_str(QUOTE_ENVELOPE_CASES).unwrap();
    for (label, reason) in [
        ("RECEIVE", QuoteReason::Receive),
        ("SEND", QuoteReason::Send),
        ("COOP_EXIT", QuoteReason::CoopExit),
        ("STATIC_DEPOSIT", QuoteReason::StaticDeposit),
    ] {
        assert_eq!(parsed["reasons"][label].as_u64(), Some(reason as u64));
    }
    for (label, role) in [
        ("ISSUER", QuoteRole::Issuer),
        ("ATTESTOR", QuoteRole::Attestor),
    ] {
        assert_eq!(parsed["roles"][label].as_u64(), Some(role as u64));
    }
}

#[test_all]
fn envelope_digest_matches_every_golden_vector() {
    for case in cases(QUOTE_ENVELOPE_CASES, "testCases") {
        let target = hex_field(&case, "target");
        if case.get("paymentHash").is_some() {
            assert_eq!(
                receive_attestor_target(&hash32(&case, "paymentHash"))
                    .unwrap()
                    .to_vec(),
                target,
                "{}",
                name(&case)
            );
        }
        let digest = envelope_digest(
            case["network"].as_u64().unwrap(),
            &hex_field(&case, "manifestHash"),
            case["reason"].as_u64().unwrap(),
            case["role"].as_u64().unwrap(),
            &target,
        )
        .unwrap_or_else(|e| panic!("{}: {e}", name(&case)));
        assert_eq!(
            hex::encode(digest),
            case["expectedDigest"].as_str().unwrap(),
            "{}",
            name(&case)
        );
    }
}

#[test_all]
fn typed_envelope_digest_matches_the_receive_attestor_vector() {
    let case = cases(QUOTE_ENVELOPE_CASES, "testCases")
        .into_iter()
        .find(|case| name(case) == "receive_attestor_regtest")
        .unwrap();
    let digest = quote_envelope_digest(
        Network::Regtest,
        &hash32(&case, "manifestHash"),
        QuoteReason::Receive,
        QuoteRole::Attestor,
        &receive_attestor_target(&hash32(&case, "paymentHash")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        hex::encode(digest),
        case["expectedDigest"].as_str().unwrap()
    );
}

#[test_all]
fn envelope_refuses_every_invalid_vector() {
    for case in cases(QUOTE_ENVELOPE_CASES, "invalidCases") {
        let expected = case["expectedError"].as_str().unwrap();
        let result = match case["call"].as_str().unwrap() {
            "quoteEnvelopeDigest" => envelope_digest(
                case["network"].as_u64().unwrap(),
                &hex_field(&case, "manifestHash"),
                case["reason"].as_u64().unwrap(),
                case["role"].as_u64().unwrap(),
                &hex_field(&case, "target"),
            ),
            // The typed API takes a [u8; 32], so a wrong-length payment hash
            // cannot reach it.
            "receiveAttestationTarget" => {
                assert_ne!(hex_field(&case, "paymentHash").len(), 32);
                continue;
            }
            other => panic!("unknown call {other}"),
        };
        let err = result.expect_err(name(&case)).to_string();
        assert!(err.contains(expected), "{}: {err}", name(&case));
    }
}

#[test_all]
fn distinct_digest_pairs_differ() {
    let digests: std::collections::HashMap<String, String> =
        cases(QUOTE_ENVELOPE_CASES, "testCases")
            .iter()
            .map(|case| {
                (
                    name(case).to_string(),
                    case["expectedDigest"].as_str().unwrap().to_string(),
                )
            })
            .collect();
    for pair in cases(QUOTE_ENVELOPE_CASES, "distinctDigestPairs") {
        let a = &digests[pair["a"].as_str().unwrap()];
        let b = &digests[pair["b"].as_str().unwrap()];
        assert_ne!(a, b, "{}", name(&pair));
    }
}

#[test_all]
fn receive_attestor_and_send_targets_match_the_vectors() {
    for case in cases(QUOTE_ENVELOPE_CASES, "targetCases") {
        let expected = case["expectedDigest"].as_str().unwrap();
        let payment_hash = || hash32(&case, "paymentHash");
        let digest = match case["call"].as_str().unwrap() {
            "receiveAttestorTarget" => receive_attestor_target(&payment_hash()).unwrap(),
            "sendTargetV2" => send_target_v2(
                &payment_hash(),
                case["paymentMsats"].as_str().unwrap().parse().unwrap(),
            )
            .unwrap(),
            // Targets of flows this crate does not quote yet.
            _ => continue,
        };
        assert_eq!(hex::encode(digest), expected, "{}", name(&case));
    }
}

// The published vectors predate send target v2. These pins were computed
// independently of spark-primitives, from the tagged-hash construction over
// ["spark","quote","target","send","v2"] with the payment hash and the
// big-endian msats; the same computation reproduces the published v1 send
// target.
#[test_all]
fn send_target_v2_matches_independently_computed_pins() {
    let payment_hash = [1u8; 32];
    for (msats, expected) in [
        (
            100_000_000,
            "a61eb23b94d015f88dcd6a30ee4080be4c0224e7ef4ced3a2a1e1665d72a25b5",
        ),
        (
            1,
            "7d0b349425364a6105f7cb9c96be541f335478e43e11c1c58d9b037e7bf10fdc",
        ),
        (
            u64::MAX,
            "774484178d4b7b1adb080877779cd4bfc69ab53a5d54e2ed3798ddea27cd3ffb",
        ),
    ] {
        assert_eq!(
            hex::encode(send_target_v2(&payment_hash, msats).unwrap()),
            expected
        );
    }
    assert!(send_target_v2(&payment_hash, 0).is_err());
}

// ─── signing digests and verification ────────────────────────────────────

fn secret(seed: u8) -> SecretKey {
    SecretKey::from_slice(&[seed; 32]).unwrap()
}

fn key(seed: u8) -> PublicKey {
    secret(seed).public_key(&Secp256k1::new())
}

fn sign(seed: u8, digest: &[u8; 32]) -> Vec<u8> {
    Secp256k1::new()
        .sign_ecdsa(&Message::from_digest(*digest), &secret(seed))
        .serialize_der()
        .to_vec()
}

fn hashable_manifest(network: Network) -> TransferManifest {
    TransferManifest {
        version: 1,
        transfer_id: "0197f9a0-0000-7000-8000-000000000001".to_string(),
        network: network.into(),
        edges: vec![edge(SSP, RECEIVER, sats(1_000))],
        ..Default::default()
    }
}

#[test_all]
fn receive_attestation_digest_composes_the_attestor_envelope() {
    let manifest = hashable_manifest(Network::Mainnet).encode_to_vec();
    let payment_hash = [7u8; 32];
    let expected = quote_envelope_digest(
        Network::Mainnet,
        &serialized_transfer_manifest_hash(&manifest).unwrap(),
        QuoteReason::Receive,
        QuoteRole::Attestor,
        &receive_attestor_target(&payment_hash).unwrap(),
    )
    .unwrap();
    assert_eq!(
        receive_attestation_digest(&manifest, &payment_hash).unwrap(),
        expected
    );
}

#[test_all]
fn send_issuance_digest_composes_the_issuer_envelope() {
    let manifest = hashable_manifest(Network::Regtest).encode_to_vec();
    let payment_hash = [7u8; 32];
    let expected = quote_envelope_digest(
        Network::Regtest,
        &serialized_transfer_manifest_hash(&manifest).unwrap(),
        QuoteReason::Send,
        QuoteRole::Issuer,
        &send_target_v2(&payment_hash, 1_000_000).unwrap(),
    )
    .unwrap();
    assert_eq!(
        send_issuance_digest(&manifest, &payment_hash, 1_000_000).unwrap(),
        expected
    );
}

#[test_all]
fn the_envelope_takes_its_network_from_the_manifest_bytes() {
    let payment_hash = [7u8; 32];
    let mainnet = hashable_manifest(Network::Mainnet).encode_to_vec();
    let regtest = hashable_manifest(Network::Regtest).encode_to_vec();
    assert_ne!(
        receive_attestation_digest(&mainnet, &payment_hash).unwrap(),
        receive_attestation_digest(&regtest, &payment_hash).unwrap()
    );
}

#[test_all]
fn verify_identity_signature_accepts_low_s_der() {
    let digest = [9u8; 32];
    let signature = sign(SSP, &digest);
    verify_identity_signature(&digest, &signature, &key(SSP)).unwrap();
}

#[test_all]
fn verify_identity_signature_refuses_another_key_or_digest() {
    let digest = [9u8; 32];
    let signature = sign(SSP, &digest);
    assert!(verify_identity_signature(&digest, &signature, &key(RECEIVER)).is_err());
    assert!(verify_identity_signature(&[8u8; 32], &signature, &key(SSP)).is_err());
}

#[test_all]
fn verify_identity_signature_refuses_high_s() {
    let digest = [9u8; 32];
    let signature = sign(SSP, &digest);
    // Negating s keeps the signature mathematically valid but non-canonical.
    let parsed = bitcoin::secp256k1::ecdsa::Signature::from_der(&signature).unwrap();
    let mut compact = parsed.serialize_compact();
    let (_, s) = compact.split_at_mut(32);
    let s_scalar = SecretKey::from_slice(s).unwrap().negate();
    s.copy_from_slice(&s_scalar.secret_bytes());
    let high_s = bitcoin::secp256k1::ecdsa::Signature::from_compact(&compact)
        .unwrap()
        .serialize_der();
    assert!(verify_identity_signature(&digest, &high_s, &key(SSP)).is_err());
}

#[test_all]
fn verify_identity_signature_refuses_compact_encoding() {
    let digest = [9u8; 32];
    let compact = bitcoin::secp256k1::ecdsa::Signature::from_der(&sign(SSP, &digest))
        .unwrap()
        .serialize_compact();
    assert!(verify_identity_signature(&digest, &compact, &key(SSP)).is_err());
}

// ─── resolution vectors ───────────────────────────────────────────────────

fn resolution_edges(case: &Value) -> Vec<ManifestEdge> {
    case["edges"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, amount)| {
            let amount = match (amount.get("sats"), amount.get("bps")) {
                (Some(flat), None) => sats(flat.as_u64().unwrap()),
                (None, Some(rate)) => bps(u32::try_from(rate.as_u64().unwrap()).unwrap()),
                _ => panic!("edge must declare exactly one of sats or bps"),
            };
            edge(SSP, u8::try_from(10 + i).unwrap(), amount)
        })
        .collect()
}

fn u64_list(case: &Value, key: &str) -> Vec<u64> {
    case[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect()
}

#[test_all]
fn resolution_matches_every_vector() {
    for case in cases(RESOLUTION_CASES, "testCases") {
        let resolved =
            resolve_manifest_edges(&resolution_edges(&case), case["gross"].as_u64().unwrap())
                .unwrap_or_else(|e| panic!("{}: {e}", name(&case)));
        let (resolved_sats, allowed_sats): (Vec<_>, Vec<_>) = resolved
            .iter()
            .map(|edge| (edge.resolved_sats, edge.allowed_sats))
            .unzip();
        assert_eq!(
            resolved_sats,
            u64_list(&case, "resolved"),
            "{}",
            name(&case)
        );
        assert_eq!(allowed_sats, u64_list(&case, "allowed"), "{}", name(&case));
    }
}

#[test_all]
fn resolution_refuses_every_invalid_vector() {
    for case in cases(RESOLUTION_CASES, "invalidCases") {
        let err = resolve_manifest_edges(&resolution_edges(&case), case["gross"].as_u64().unwrap())
            .expect_err(name(&case))
            .to_string();
        let expected = case["expectedError"].as_str().unwrap();
        assert!(err.contains(expected), "{}: {err}", name(&case));
    }
}

#[test_all]
fn allocation_places_dust_by_largest_remainder() {
    let rates = |rates: &[u32]| -> Vec<ManifestEdge> {
        rates
            .iter()
            .zip(10u8..)
            .map(|(rate, receiver)| edge(SSP, receiver, bps(*rate)))
            .collect()
    };
    // (gross, edges, allocation): each sums to the gross, and every edge stays
    // within its allowed bound.
    let cases: [(u64, Vec<ManifestEdge>, Vec<u64>); 6] = [
        (1_000, rates(&[6_000, 4_000]), vec![600, 400]),
        (999, rates(&[6_000, 4_000]), vec![599, 400]),
        (999, rates(&[9_700, 100, 150, 50]), vec![969, 10, 15, 5]),
        (40, rates(&[9_999, 1]), vec![40, 0]),
        // Equal remainders: the earlier edge takes the sat.
        (3, rates(&[5_000, 5_000]), vec![2, 1]),
        (
            1_000,
            vec![
                edge(SSP, 10, sats(100)),
                edge(SSP, 11, bps(6_000)),
                edge(SSP, 12, bps(4_000)),
            ],
            vec![100, 540, 360],
        ),
    ];
    for (gross, edges, expected) in cases {
        let allocated = allocate_manifest_edges(&edges, gross).unwrap();
        assert_eq!(allocated, expected);
        assert_eq!(allocated.iter().sum::<u64>(), gross);
        let resolved = resolve_manifest_edges(&edges, gross).unwrap();
        for (sats, edge) in allocated.iter().zip(resolved) {
            assert!((edge.resolved_sats..=edge.allowed_sats).contains(sats));
        }
    }
}

// ─── receive validation ───────────────────────────────────────────────────

const SSP: u8 = 1;
const RECEIVER: u8 = 2;
const AFFILIATE: u8 = 3;
const PARTNER: u8 = 4;
const SENDER: u8 = 5;
const STRANGER: u8 = 6;

fn sats(value: u64) -> ManifestAmount {
    ManifestAmount {
        amount: Some(Amount::Sats(value)),
    }
}

fn bps(value: u32) -> ManifestAmount {
    ManifestAmount {
        amount: Some(Amount::Bps(value)),
    }
}

fn edge(sender: u8, receiver: u8, amount: ManifestAmount) -> ManifestEdge {
    ManifestEdge {
        sender_identity_public_key: key(sender).serialize().to_vec(),
        receiver_identity_public_key: key(receiver).serialize().to_vec(),
        amount: Some(amount),
    }
}

fn fee(receiver: u8, amount: ManifestAmount, role: FeeRole) -> FeeComponent {
    FeeComponent {
        source: FeeSource::PartnerMarkup.into(),
        role: role.into(),
        amount: Some(amount),
        receiver_identity_public_key: key(receiver).serialize().to_vec(),
    }
}

fn manifest(edges: Vec<ManifestEdge>, fees: Vec<FeeComponent>) -> TransferManifest {
    TransferManifest {
        version: 1,
        transfer_id: "0197f9a0-0000-7000-8000-000000000001".to_string(),
        network: Network::Regtest.into(),
        edges,
        fees,
        ..Default::default()
    }
}

fn validate(
    quoted: &TransferManifest,
    amount_sats: u64,
    basis: ReceiveAmountBasis,
) -> Result<(), QuoteError> {
    validate_quoted_receive_manifest(quoted, amount_sats, basis, &key(RECEIVER), &key(RECEIVER))
}

#[track_caller]
fn assert_refused(result: Result<(), QuoteError>, needle: &str) {
    let err = result.expect_err("expected a refusal").to_string();
    assert!(err.contains(needle), "{err:?} does not mention {needle:?}");
}

fn feeless() -> TransferManifest {
    manifest(vec![edge(SSP, RECEIVER, sats(100_000))], vec![])
}

fn markup_fees(affiliate: u64, partner: u64, ssp: u64) -> Vec<FeeComponent> {
    vec![
        fee(AFFILIATE, sats(affiliate), FeeRole::Affiliate),
        fee(PARTNER, sats(partner), FeeRole::Partner),
        fee(SSP, sats(ssp), FeeRole::Ls),
    ]
}

// An attributed 200 bps markup on a net basis: the receiver nets what it
// asked for and the payer funds 102_000.
fn fee_bearing_net() -> TransferManifest {
    manifest(
        vec![
            edge(SSP, RECEIVER, sats(100_000)),
            edge(SSP, AFFILIATE, sats(1_000)),
            edge(SSP, PARTNER, sats(700)),
            edge(SSP, SSP, sats(300)),
        ],
        markup_fees(1_000, 700, 300),
    )
}

// The same markup on a gross basis: the invoice is exactly 100_000 and the
// receiver absorbs the 2_000.
fn fee_bearing_gross() -> TransferManifest {
    manifest(
        vec![
            edge(SSP, RECEIVER, sats(98_000)),
            edge(SSP, AFFILIATE, sats(1_000)),
            edge(SSP, PARTNER, sats(700)),
            edge(SSP, SSP, sats(300)),
        ],
        markup_fees(1_000, 700, 300),
    )
}

#[test_all]
fn receive_accepts_a_feeless_quote_on_either_basis() {
    validate(&feeless(), 100_000, ReceiveAmountBasis::Net).unwrap();
    validate(&feeless(), 100_000, ReceiveAmountBasis::Gross).unwrap();
}

#[test_all]
fn receive_accepts_a_net_quote_whose_gross_exceeds_the_request() {
    let quoted = fee_bearing_net();
    assert_eq!(manifest_gross_sats(&quoted).unwrap(), 102_000);
    assert_eq!(manifest_fee_sats(&quoted).unwrap(), 2_000);
    assert_eq!(
        manifest_net_sats_for(&quoted, &key(RECEIVER)).unwrap(),
        100_000
    );
    validate(&quoted, 100_000, ReceiveAmountBasis::Net).unwrap();
}

#[test_all]
fn receive_accepts_a_gross_quote_where_the_receiver_nets_less() {
    let quoted = fee_bearing_gross();
    assert_eq!(manifest_gross_sats(&quoted).unwrap(), 100_000);
    assert_eq!(
        manifest_net_sats_for(&quoted, &key(RECEIVER)).unwrap(),
        98_000
    );
    validate(&quoted, 100_000, ReceiveAmountBasis::Gross).unwrap();
}

#[test_all]
fn receive_refuses_a_fee_bearing_quote_checked_on_the_other_basis() {
    assert_refused(
        validate(&fee_bearing_net(), 100_000, ReceiveAmountBasis::Gross),
        "does not match the requested amount",
    );
    assert_refused(
        validate(&fee_bearing_gross(), 100_000, ReceiveAmountBasis::Net),
        "does not match the requested amount",
    );
}

#[test_all]
fn receive_refuses_a_quote_that_pays_somebody_else() {
    let elsewhere = manifest(vec![edge(SSP, AFFILIATE, sats(100_000))], vec![]);
    assert_refused(
        validate(&elsewhere, 100_000, ReceiveAmountBasis::Net),
        "pays the receiver on 0 edges",
    );
}

#[test_all]
fn receive_nets_out_a_fee_credited_back_to_the_receiver() {
    let self_affiliate = manifest(
        vec![
            edge(SSP, RECEIVER, sats(101_000)),
            edge(SSP, SSP, sats(1_000)),
        ],
        vec![
            fee(RECEIVER, sats(1_000), FeeRole::Affiliate),
            fee(SSP, sats(1_000), FeeRole::Ls),
        ],
    );
    validate(&self_affiliate, 100_000, ReceiveAmountBasis::Net).unwrap();
    assert_refused(
        validate(&self_affiliate, 101_000, ReceiveAmountBasis::Net),
        "does not match the requested amount",
    );
}

#[test_all]
fn receive_refuses_an_amount_the_quote_does_not_cover() {
    assert_refused(
        validate(&feeless(), 99_999, ReceiveAmountBasis::Net),
        "does not match the requested amount",
    );
}

#[test_all]
fn receive_sums_every_fee_credited_to_the_same_key() {
    let two_fees_each = manifest(
        vec![
            edge(SSP, RECEIVER, sats(101_500)),
            edge(SSP, SSP, sats(1_500)),
        ],
        vec![
            fee(RECEIVER, sats(1_000), FeeRole::Affiliate),
            fee(RECEIVER, sats(500), FeeRole::Partner),
            fee(SSP, sats(1_000), FeeRole::Ls),
            fee(SSP, sats(500), FeeRole::Partner),
        ],
    );
    validate(&two_fees_each, 100_000, ReceiveAmountBasis::Net).unwrap();
    assert_refused(
        validate(&two_fees_each, 100_500, ReceiveAmountBasis::Net),
        "does not match the requested amount",
    );
}

#[test_all]
fn receive_refuses_an_edge_to_a_key_owed_no_fee() {
    let stranger = manifest(
        vec![
            edge(SSP, RECEIVER, sats(100_000)),
            edge(SSP, AFFILIATE, sats(900_000)),
        ],
        vec![],
    );
    assert_refused(
        validate(&stranger, 100_000, ReceiveAmountBasis::Net),
        "pays 900000 sats to a key owed 0 in fees",
    );
}

#[test_all]
fn receive_refuses_an_edge_that_overpays_its_fee() {
    let overpaid = manifest(
        vec![
            edge(SSP, RECEIVER, sats(100_000)),
            edge(SSP, AFFILIATE, sats(5_000)),
        ],
        vec![fee(AFFILIATE, sats(1_000), FeeRole::Affiliate)],
    );
    assert_refused(
        validate(&overpaid, 100_000, ReceiveAmountBasis::Net),
        "pays 5000 sats to a key owed 1000 in fees",
    );
}

#[test_all]
fn receive_refuses_more_than_one_edge_to_the_receiver() {
    let split = manifest(
        vec![
            edge(SSP, RECEIVER, sats(60_000)),
            edge(SSP, RECEIVER, sats(40_000)),
        ],
        vec![],
    );
    assert_refused(
        validate(&split, 100_000, ReceiveAmountBasis::Net),
        "pays the receiver on 2 edges",
    );
}

#[test_all]
fn receive_refuses_an_edge_with_no_amount_rather_than_reading_zero() {
    let mut unset = feeless();
    if let Some(edge) = unset.edges.first_mut() {
        edge.amount = None;
    }
    assert_refused(
        validate(&unset, 100_000, ReceiveAmountBasis::Net),
        "has no amount",
    );
}

#[test_all]
fn receive_refuses_a_fee_no_edge_funds() {
    let unfunded = manifest(
        vec![edge(SSP, RECEIVER, sats(100_000))],
        vec![fee(AFFILIATE, sats(1_000), FeeRole::Affiliate)],
    );
    assert_refused(
        validate(&unfunded, 100_000, ReceiveAmountBasis::Net),
        "does not conserve value",
    );
}

#[test_all]
fn receive_refuses_a_manifest_debiting_the_receiving_wallet() {
    let debits = manifest(
        vec![
            edge(SSP, RECEIVER, sats(100_000)),
            edge(RECEIVER, SSP, sats(2_000)),
        ],
        vec![fee(SSP, sats(2_000), FeeRole::Ls)],
    );
    // The attestor defaults to the receiver on a self-receive.
    assert_refused(
        validate(&debits, 100_000, ReceiveAmountBasis::Net),
        "debits the attesting wallet",
    );
}

#[test_all]
fn delegated_receive_refuses_a_manifest_debiting_either_party() {
    let debits_payee = manifest(
        vec![
            edge(SSP, RECEIVER, sats(100_000)),
            edge(RECEIVER, SSP, sats(2_000)),
        ],
        vec![fee(SSP, sats(2_000), FeeRole::Ls)],
    );
    assert_refused(
        validate_quoted_receive_manifest(
            &debits_payee,
            100_000,
            ReceiveAmountBasis::Net,
            &key(RECEIVER),
            &key(PARTNER),
        ),
        "debits the receiving wallet",
    );
    let debits_attestor = manifest(
        vec![
            edge(SSP, RECEIVER, sats(100_000)),
            edge(PARTNER, SSP, sats(2_000)),
        ],
        vec![fee(SSP, sats(2_000), FeeRole::Ls)],
    );
    assert_refused(
        validate_quoted_receive_manifest(
            &debits_attestor,
            100_000,
            ReceiveAmountBasis::Net,
            &key(RECEIVER),
            &key(PARTNER),
        ),
        "debits the attesting wallet",
    );
}

#[test_all]
fn delegated_receive_accepts_a_quote_naming_the_payee() {
    validate_quoted_receive_manifest(
        &fee_bearing_net(),
        100_000,
        ReceiveAmountBasis::Net,
        &key(RECEIVER),
        &key(PARTNER),
    )
    .unwrap();
}

#[test_all]
fn receive_refuses_a_total_that_overflows() {
    let half = u64::MAX / 2 + 1;
    let overflowing = manifest(
        vec![
            edge(SSP, RECEIVER, sats(half)),
            edge(SSP, AFFILIATE, sats(half)),
        ],
        vec![fee(AFFILIATE, sats(half), FeeRole::Affiliate)],
    );
    assert_refused(
        validate(&overflowing, half, ReceiveAmountBasis::Net),
        "overflows",
    );
}

#[test_all]
fn receive_refuses_a_quote_leaving_the_receiver_nothing() {
    let consumed = manifest(
        vec![edge(SSP, RECEIVER, sats(1_000))],
        vec![fee(RECEIVER, sats(1_000), FeeRole::Ls)],
    );
    assert_refused(
        validate(&consumed, 1_000, ReceiveAmountBasis::Gross),
        "leaves the receiver nothing",
    );
}

// ─── amountless (bps) receive validation ─────────────────────────────────

fn amountless_feeless() -> TransferManifest {
    manifest(vec![edge(SSP, RECEIVER, bps(10_000))], vec![])
}

#[test_all]
fn amountless_accepts_a_feeless_manifest_whose_rates_partition() {
    validate(&amountless_feeless(), 0, ReceiveAmountBasis::Gross).unwrap();
}

#[test_all]
fn amountless_accepts_a_fee_bearing_manifest_whose_rates_partition() {
    let quoted = manifest(
        vec![
            edge(SSP, RECEIVER, bps(9_800)),
            edge(SSP, AFFILIATE, bps(100)),
            edge(SSP, PARTNER, bps(70)),
            edge(SSP, SSP, bps(30)),
        ],
        vec![
            fee(AFFILIATE, bps(100), FeeRole::Affiliate),
            fee(PARTNER, bps(70), FeeRole::Partner),
            fee(SSP, bps(30), FeeRole::Ls),
        ],
    );
    assert_eq!(manifest_denomination(&quoted), ManifestDenomination::Bps);
    validate(&quoted, 0, ReceiveAmountBasis::Gross).unwrap();
}

#[test_all]
fn amountless_refuses_rates_that_do_not_partition() {
    let short = manifest(
        vec![
            edge(SSP, RECEIVER, bps(6_000)),
            edge(SSP, AFFILIATE, bps(3_000)),
        ],
        vec![],
    );
    assert_refused(
        validate(&short, 0, ReceiveAmountBasis::Gross),
        "do not partition",
    );
}

#[test_all]
fn amountless_refuses_a_net_basis() {
    assert_refused(
        validate(&amountless_feeless(), 0, ReceiveAmountBasis::Net),
        "no net to pin",
    );
}

#[test_all]
fn denomination_comes_from_the_edges_not_a_fee_annotation() {
    let flat_with_rate_fee = manifest(
        vec![
            edge(SSP, RECEIVER, sats(98_000)),
            edge(SSP, SSP, sats(2_000)),
        ],
        vec![fee(SSP, bps(200), FeeRole::Ls)],
    );
    assert_eq!(
        manifest_denomination(&flat_with_rate_fee),
        ManifestDenomination::Sats
    );
    let err = validate(&flat_with_rate_fee, 100_000, ReceiveAmountBasis::Gross);
    assert!(!format!("{err:?}").contains("denomination does not match"));
}

#[test_all]
fn receive_refuses_a_shape_mismatch_between_amount_and_denomination() {
    assert_refused(
        validate(&feeless(), 0, ReceiveAmountBasis::Gross),
        "denomination does not match",
    );
    assert_refused(
        validate(&amountless_feeless(), 100_000, ReceiveAmountBasis::Gross),
        "denomination does not match",
    );
}

#[test_all]
fn amountless_refuses_a_rate_edge_owed_a_different_rate() {
    let overpaid = manifest(
        vec![
            edge(SSP, RECEIVER, bps(9_800)),
            edge(SSP, AFFILIATE, bps(200)),
        ],
        vec![fee(AFFILIATE, bps(100), FeeRole::Affiliate)],
    );
    assert_refused(validate(&overpaid, 0, ReceiveAmountBasis::Gross), "owed");
}

#[test_all]
fn amountless_refuses_a_manifest_paying_the_receiver_no_edge() {
    let elsewhere = manifest(
        vec![edge(SSP, PARTNER, bps(10_000))],
        vec![fee(PARTNER, bps(10_000), FeeRole::Ls)],
    );
    assert_refused(
        validate(&elsewhere, 0, ReceiveAmountBasis::Gross),
        "pays the receiver on 0 edges",
    );
}

#[test_all]
fn amountless_refuses_a_zero_receiver_rate() {
    let nothing = manifest(
        vec![edge(SSP, RECEIVER, bps(0)), edge(SSP, SSP, bps(10_000))],
        vec![fee(SSP, bps(10_000), FeeRole::Ls)],
    );
    assert_refused(
        validate(&nothing, 0, ReceiveAmountBasis::Gross),
        "leaves the receiver nothing",
    );
}

#[test_all]
fn amountless_refuses_two_edges_to_the_receiver() {
    let split = manifest(
        vec![
            edge(SSP, RECEIVER, bps(5_000)),
            edge(PARTNER, RECEIVER, bps(5_000)),
        ],
        vec![],
    );
    assert_refused(
        validate(&split, 0, ReceiveAmountBasis::Gross),
        "pays the receiver on 2 edges",
    );
}

#[test_all]
fn amountless_refuses_a_fee_crediting_back_the_whole_receiver_rate() {
    let consumed = manifest(
        vec![edge(SSP, RECEIVER, bps(10_000))],
        vec![fee(RECEIVER, bps(10_000), FeeRole::Ls)],
    );
    assert_refused(
        validate(&consumed, 0, ReceiveAmountBasis::Gross),
        "leaves the receiver nothing",
    );
}

#[test_all]
fn amountless_refuses_an_undeclared_flat_leg_beside_the_rates() {
    let unbacked = manifest(
        vec![
            edge(SSP, RECEIVER, bps(10_000)),
            edge(SSP, SSP, sats(1_000_000)),
        ],
        vec![],
    );
    assert_refused(
        validate(&unbacked, 0, ReceiveAmountBasis::Gross),
        "pays 1000000 sats to a key owed 0 in fees",
    );
}

#[test_all]
fn amountless_refuses_a_flat_leg_exceeding_its_declared_fee() {
    let underdeclared = manifest(
        vec![
            edge(SSP, RECEIVER, bps(10_000)),
            edge(SSP, SSP, sats(5_000)),
        ],
        vec![fee(SSP, sats(1), FeeRole::Ls)],
    );
    assert_refused(
        validate(&underdeclared, 0, ReceiveAmountBasis::Gross),
        "pays 5000 sats to a key owed 1 in fees",
    );
}

#[test_all]
fn amountless_accepts_a_flat_leg_matched_by_its_declared_fee() {
    let backed = manifest(
        vec![
            edge(SSP, RECEIVER, bps(10_000)),
            edge(SSP, SSP, sats(5_000)),
        ],
        vec![fee(SSP, sats(5_000), FeeRole::Ls)],
    );
    validate(&backed, 0, ReceiveAmountBasis::Gross).unwrap();
}

#[test_all]
fn amountless_refuses_a_rate_fee_crediting_a_key_with_no_edge() {
    let uncredited = manifest(
        vec![edge(SSP, RECEIVER, bps(10_000))],
        vec![fee(AFFILIATE, bps(200), FeeRole::Affiliate)],
    );
    assert_refused(
        validate(&uncredited, 0, ReceiveAmountBasis::Gross),
        "credits a key holding no matching edge",
    );
}

#[test_all]
fn amountless_refuses_a_flat_fee_on_a_payee_paid_only_a_rate() {
    let crossed = manifest(
        vec![
            edge(SSP, RECEIVER, bps(9_800)),
            edge(SSP, AFFILIATE, bps(200)),
        ],
        vec![
            fee(AFFILIATE, bps(200), FeeRole::Affiliate),
            fee(AFFILIATE, sats(50_000), FeeRole::Ls),
        ],
    );
    assert_refused(
        validate(&crossed, 0, ReceiveAmountBasis::Gross),
        "credits a key holding no matching edge",
    );
}

#[test_all]
fn amountless_accepts_a_payee_holding_a_rate_and_a_flat_edge_each_declared() {
    let both = manifest(
        vec![
            edge(SSP, RECEIVER, bps(9_970)),
            edge(SSP, SSP, bps(30)),
            edge(SSP, SSP, sats(5_000)),
        ],
        vec![
            fee(SSP, bps(30), FeeRole::Ls),
            fee(SSP, sats(5_000), FeeRole::Ls),
        ],
    );
    validate(&both, 0, ReceiveAmountBasis::Gross).unwrap();
}

#[test_all]
fn amountless_refuses_that_payee_when_its_flat_fee_understates_its_flat_edge() {
    let short = manifest(
        vec![
            edge(SSP, RECEIVER, bps(9_970)),
            edge(SSP, SSP, bps(30)),
            edge(SSP, SSP, sats(5_000)),
        ],
        vec![
            fee(SSP, bps(30), FeeRole::Ls),
            fee(SSP, sats(4_999), FeeRole::Ls),
        ],
    );
    assert_refused(
        validate(&short, 0, ReceiveAmountBasis::Gross),
        "pays 5000 sats to a key owed 4999 in fees",
    );
}

#[test_all]
fn amountless_refuses_two_edges_to_one_payee_backed_by_one_fee() {
    let doubled = manifest(
        vec![
            edge(SSP, RECEIVER, bps(9_800)),
            edge(SSP, AFFILIATE, bps(100)),
            edge(PARTNER, AFFILIATE, bps(100)),
        ],
        vec![fee(AFFILIATE, bps(100), FeeRole::Affiliate)],
    );
    assert_refused(
        validate(&doubled, 0, ReceiveAmountBasis::Gross),
        "pays 200 bps to a key owed 100 in fees",
    );
}

#[test_all]
fn amountless_accepts_two_edges_to_one_payee_when_both_are_declared() {
    let declared = manifest(
        vec![
            edge(SSP, RECEIVER, bps(9_800)),
            edge(SSP, AFFILIATE, bps(100)),
            edge(PARTNER, AFFILIATE, bps(100)),
        ],
        vec![
            fee(AFFILIATE, bps(100), FeeRole::Affiliate),
            fee(AFFILIATE, bps(100), FeeRole::Ls),
        ],
    );
    validate(&declared, 0, ReceiveAmountBasis::Gross).unwrap();
}

#[test_all]
fn amountless_names_the_missing_receiver_ahead_of_a_failed_partition() {
    let elsewhere = manifest(
        vec![edge(SSP, SSP, bps(9_000))],
        vec![fee(SSP, bps(9_000), FeeRole::Ls)],
    );
    assert_refused(
        validate(&elsewhere, 0, ReceiveAmountBasis::Gross),
        "pays the receiver on 0 edges",
    );
}

#[test_all]
fn amountless_refuses_an_edge_with_no_amount() {
    let mut unset = manifest(
        vec![edge(SSP, RECEIVER, bps(10_000)), edge(SSP, SSP, bps(0))],
        vec![],
    );
    if let Some(edge) = unset.edges.get_mut(1) {
        edge.amount = None;
    }
    assert_refused(
        validate(&unset, 0, ReceiveAmountBasis::Gross),
        "has no amount",
    );
}

#[test_all]
fn amountless_refuses_a_rate_edge_debiting_the_attesting_wallet() {
    let debits = manifest(
        vec![
            edge(SSP, RECEIVER, bps(9_800)),
            edge(RECEIVER, SSP, bps(200)),
        ],
        vec![fee(SSP, bps(200), FeeRole::Ls)],
    );
    assert_refused(
        validate(&debits, 0, ReceiveAmountBasis::Gross),
        "debits the attesting wallet",
    );
}

// ─── send validation ──────────────────────────────────────────────────────

fn send_manifest(edges: Vec<ManifestEdge>, fees: Vec<FeeComponent>) -> TransferManifest {
    manifest(edges, fees)
}

fn send_edge(receiver: u8, amount: u64) -> ManifestEdge {
    edge(SENDER, receiver, sats(amount))
}

fn validate_send(quoted: &TransferManifest) -> Result<(), QuoteError> {
    validate_quoted_send_manifest(quoted, 10_000, &key(SENDER))
}

#[test_all]
fn send_accepts_the_three_bucket_split_a_fee_bearing_send_quotes() {
    let quoted = send_manifest(
        vec![
            send_edge(SSP, 10_500),
            send_edge(PARTNER, 1_334),
            send_edge(AFFILIATE, 1_000),
        ],
        vec![fee(PARTNER, sats(1_334), FeeRole::Partner)],
    );
    validate_send(&quoted).unwrap();
}

#[test_all]
fn send_accepts_the_single_edge_an_unattributed_quote_carries() {
    validate_send(&send_manifest(vec![send_edge(SSP, 10_500)], vec![])).unwrap();
}

// The receive conservation check would refuse every send quote: the SSP's own
// edge carries the payment and its fee, which no fee component declares.
#[test_all]
fn send_is_not_held_to_receive_conservation() {
    let quoted = send_manifest(
        vec![send_edge(SSP, 10_500), send_edge(PARTNER, 1_334)],
        vec![fee(PARTNER, sats(1_334), FeeRole::Partner)],
    );
    validate_send(&quoted).unwrap();
    let gross = manifest_gross_sats(&quoted).unwrap();
    let fees = manifest_fee_sats(&quoted).unwrap();
    assert_ne!(gross, 10_000 + fees);
}

#[test_all]
fn send_refuses_an_edge_sent_by_another_wallet() {
    let quoted = send_manifest(vec![edge(STRANGER, SSP, sats(10_500))], vec![]);
    assert_refused(validate_send(&quoted), "sent by another wallet");
}

#[test_all]
fn send_refuses_an_edge_paying_the_sending_wallet() {
    let quoted = send_manifest(
        vec![send_edge(SSP, 10_500), send_edge(SENDER, 1_000)],
        vec![],
    );
    assert_refused(validate_send(&quoted), "pays the sending wallet");
}

#[test_all]
fn send_refuses_a_repeated_receiver() {
    let quoted = send_manifest(vec![send_edge(SSP, 10_500), send_edge(SSP, 1_000)], vec![]);
    assert_refused(validate_send(&quoted), "repeats a receiver");
}

#[test_all]
fn send_refuses_a_fee_crediting_a_key_with_no_edge() {
    let quoted = send_manifest(
        vec![send_edge(SSP, 10_500)],
        vec![fee(STRANGER, sats(500), FeeRole::Partner)],
    );
    assert_refused(validate_send(&quoted), "credits a key holding no edge");
}

#[test_all]
fn send_refuses_a_manifest_not_covering_the_payment() {
    let quoted = send_manifest(vec![send_edge(SSP, 9_999)], vec![]);
    assert_refused(validate_send(&quoted), "does not cover the payment amount");
}

#[test_all]
fn send_refuses_a_rate_denominated_quote() {
    let quoted = send_manifest(vec![edge(SENDER, SSP, bps(10_000))], vec![]);
    assert_refused(validate_send(&quoted), "must be denominated in satoshis");
}

fn bucket_pairs(quoted: &TransferManifest) -> Vec<(PublicKey, u64)> {
    manifest_send_buckets(quoted)
        .unwrap()
        .into_iter()
        .map(|bucket| (bucket.receiver_identity_public_key, bucket.amount_sats))
        .collect()
}

// Key and amount together: amounts alone still sum correctly when two
// receivers' buckets are swapped.
#[test_all]
fn send_buckets_fund_one_bucket_per_receiver() {
    let quoted = send_manifest(
        vec![
            send_edge(SSP, 10_500),
            send_edge(PARTNER, 1_334),
            send_edge(AFFILIATE, 1_000),
        ],
        vec![],
    );
    assert_eq!(
        bucket_pairs(&quoted),
        vec![
            (key(SSP), 10_500),
            (key(PARTNER), 1_334),
            (key(AFFILIATE), 1_000)
        ]
    );
}

#[test_all]
fn send_buckets_sum_a_receiver_holding_several_edges() {
    let quoted = send_manifest(
        vec![
            send_edge(SSP, 10_500),
            send_edge(PARTNER, 1_334),
            send_edge(SSP, 200),
        ],
        vec![],
    );
    assert_eq!(
        bucket_pairs(&quoted),
        vec![(key(SSP), 10_700), (key(PARTNER), 1_334)]
    );
}

#[test_all]
fn send_buckets_fund_the_whole_edge_of_a_fee_annotated_key() {
    let quoted = send_manifest(
        vec![send_edge(SSP, 10_500), send_edge(PARTNER, 1_334)],
        vec![fee(PARTNER, sats(1_334), FeeRole::Partner)],
    );
    assert_eq!(
        bucket_pairs(&quoted),
        vec![(key(SSP), 10_500), (key(PARTNER), 1_334)]
    );
}

#[test_all]
fn send_buckets_refuse_a_zero_amount_edge() {
    let quoted = send_manifest(vec![send_edge(SSP, 10_500), send_edge(PARTNER, 0)], vec![]);
    let err = manifest_send_buckets(&quoted).unwrap_err().to_string();
    assert!(err.contains("not a positive whole number of sats"), "{err}");
}

#[test_all]
fn send_buckets_refuse_a_rate_edge() {
    let quoted = send_manifest(vec![edge(SENDER, SSP, bps(10_000))], vec![]);
    let err = manifest_send_buckets(&quoted).unwrap_err().to_string();
    assert!(err.contains("not in sats"), "{err}");
}

#[test_all]
fn receive_quote_fee_is_the_gross_less_the_receiver_net() {
    assert_eq!(
        receive_quote_fee(&fee_bearing_net(), &key(RECEIVER)).unwrap(),
        ReceiveQuoteFee {
            flat_sats: 2_000,
            rate_bps: 0
        }
    );
    assert_eq!(
        receive_quote_fee(&feeless(), &key(RECEIVER)).unwrap(),
        ReceiveQuoteFee::default()
    );
}

#[test_all]
fn receive_quote_fee_on_an_amountless_quote_is_a_rate_plus_flat_legs() {
    let quoted = manifest(
        vec![
            edge(SSP, RECEIVER, bps(9_800)),
            edge(SSP, AFFILIATE, bps(200)),
            edge(SSP, SSP, sats(5)),
        ],
        vec![
            fee(AFFILIATE, bps(200), FeeRole::Affiliate),
            fee(SSP, sats(5), FeeRole::Ls),
        ],
    );
    validate(&quoted, 0, ReceiveAmountBasis::Gross).unwrap();
    assert_eq!(
        receive_quote_fee(&quoted, &key(RECEIVER)).unwrap(),
        ReceiveQuoteFee {
            flat_sats: 5,
            rate_bps: 200
        }
    );
}
