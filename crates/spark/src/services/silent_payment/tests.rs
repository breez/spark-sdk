use std::str::FromStr;

use bitcoin::{
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, WScriptHash, Witness,
    absolute::LockTime, hashes::Hash, transaction::Version,
};
use macros::test_all;

use super::*;
use crate::{Network, services::silent_payment::dleq::tests::generate_proof};

#[cfg(feature = "browser-tests")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

struct Vector {
    comment: &'static str,
    inputs: &'static [(&'static str, u32, &'static str)],
    address: &'static str,
    output: &'static str,
}

// Sending cases of the BIP-352 test vectors in which every input counts and
// one recipient is paid.
const VECTORS: &[Vector] = &[
    Vector {
        comment: "Simple send: two inputs",
        inputs: &[
            (
                "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16",
                0,
                "eadc78165ff1f8ea94ad7cfdc54990738a4c53f6e0507b42154201b8e5dff3b1",
            ),
            (
                "a1075db55d416d3ca199f55b6084e2115b9345e16c5cf302fc80e9d5fbf5d48d",
                0,
                "93f5ed907ad5b2bdbbdcb5d9116ebc0a4e1f92f910d5260237fa45a9408aad16",
            ),
        ],
        address: "sp1qqgste7k9hx0qftg6qmwlkqtwuy6cycyavzmzj85c6qdfhjdpdjtdgqjuexzk6murw56suy3e0rd2cgqvycxttddwsvgxe2usfpxumr70xc9pkqwv",
        output: "3e9fce73d4e77a4809908e3c3a2e54ee147b9312dc5044a193d1fc85de46e3c1",
    },
    Vector {
        comment: "Simple send: two inputs from the same transaction",
        inputs: &[
            (
                "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16",
                3,
                "eadc78165ff1f8ea94ad7cfdc54990738a4c53f6e0507b42154201b8e5dff3b1",
            ),
            (
                "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16",
                7,
                "93f5ed907ad5b2bdbbdcb5d9116ebc0a4e1f92f910d5260237fa45a9408aad16",
            ),
        ],
        address: "sp1qqgste7k9hx0qftg6qmwlkqtwuy6cycyavzmzj85c6qdfhjdpdjtdgqjuexzk6murw56suy3e0rd2cgqvycxttddwsvgxe2usfpxumr70xc9pkqwv",
        output: "79e71baa2ba3fc66396de3a04f168c7bf24d6870ec88ca877754790c1db357b6",
    },
    Vector {
        comment: "Outpoint ordering byte-lexicographically vs. vout-integer",
        inputs: &[
            (
                "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16",
                1,
                "eadc78165ff1f8ea94ad7cfdc54990738a4c53f6e0507b42154201b8e5dff3b1",
            ),
            (
                "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16",
                256,
                "93f5ed907ad5b2bdbbdcb5d9116ebc0a4e1f92f910d5260237fa45a9408aad16",
            ),
        ],
        address: "sp1qqgste7k9hx0qftg6qmwlkqtwuy6cycyavzmzj85c6qdfhjdpdjtdgqjuexzk6murw56suy3e0rd2cgqvycxttddwsvgxe2usfpxumr70xc9pkqwv",
        output: "a85ef8701394b517a4b35217c4bd37ac01ebeed4b008f8d0879f9e09ba95319c",
    },
    Vector {
        comment: "Single recipient: multiple UTXOs from the same public key",
        inputs: &[
            (
                "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16",
                0,
                "eadc78165ff1f8ea94ad7cfdc54990738a4c53f6e0507b42154201b8e5dff3b1",
            ),
            (
                "a1075db55d416d3ca199f55b6084e2115b9345e16c5cf302fc80e9d5fbf5d48d",
                0,
                "eadc78165ff1f8ea94ad7cfdc54990738a4c53f6e0507b42154201b8e5dff3b1",
            ),
        ],
        address: "sp1qqgste7k9hx0qftg6qmwlkqtwuy6cycyavzmzj85c6qdfhjdpdjtdgqjuexzk6murw56suy3e0rd2cgqvycxttddwsvgxe2usfpxumr70xc9pkqwv",
        output: "548ae55c8eec1e736e8d3e520f011f1f42a56d166116ad210b3937599f87f566",
    },
    Vector {
        comment: "Receiving with labels: label with even parity",
        inputs: &[
            (
                "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16",
                0,
                "eadc78165ff1f8ea94ad7cfdc54990738a4c53f6e0507b42154201b8e5dff3b1",
            ),
            (
                "a1075db55d416d3ca199f55b6084e2115b9345e16c5cf302fc80e9d5fbf5d48d",
                0,
                "0378e95685b74565fa56751b84a32dfd18545d10d691641b8372e32164fad66a",
            ),
        ],
        address: "sp1qqgste7k9hx0qftg6qmwlkqtwuy6cycyavzmzj85c6qdfhjdpdjtdgqjex54dmqmmv6rw353tsuqhs99ydvadxzrsy9nuvk74epvee55drs734pqq",
        output: "d014d4860f67d607d60b1af70e0ee236b99658b61bb769832acbbe87c374439a",
    },
    Vector {
        comment: "Receiving with labels: label with odd parity",
        inputs: &[
            (
                "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16",
                0,
                "eadc78165ff1f8ea94ad7cfdc54990738a4c53f6e0507b42154201b8e5dff3b1",
            ),
            (
                "a1075db55d416d3ca199f55b6084e2115b9345e16c5cf302fc80e9d5fbf5d48d",
                0,
                "0378e95685b74565fa56751b84a32dfd18545d10d691641b8372e32164fad66a",
            ),
        ],
        address: "sp1qqgste7k9hx0qftg6qmwlkqtwuy6cycyavzmzj85c6qdfhjdpdjtdgqsg59z2rppn4qlkx0yz9sdltmjv3j8zgcqadjn4ug98m3t6plujsq9qvu5n",
        output: "67626aebb3c4307cf0f6c39ca23247598fabf675ab783292eb2f81ae75ad1f8c",
    },
    Vector {
        comment: "Receiving with labels: large label integer",
        inputs: &[
            (
                "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16",
                0,
                "eadc78165ff1f8ea94ad7cfdc54990738a4c53f6e0507b42154201b8e5dff3b1",
            ),
            (
                "a1075db55d416d3ca199f55b6084e2115b9345e16c5cf302fc80e9d5fbf5d48d",
                0,
                "0378e95685b74565fa56751b84a32dfd18545d10d691641b8372e32164fad66a",
            ),
        ],
        address: "sp1qqgste7k9hx0qftg6qmwlkqtwuy6cycyavzmzj85c6qdfhjdpdjtdgq7c2zfthc6x3a5yecwc52nxa0kfd20xuz08zyrjpfw4l2j257yq6qgnkdh5",
        output: "7efa60ce78ac343df8a013a2027c6c5ef29f9502edcbd769d2c21717fecc5951",
    },
    Vector {
        comment: "Input keys intermediate sum is zero but final sum is non-zero",
        inputs: &[
            (
                "3a286147b25e16ae80aff406f2673c6e565418c40f45c071245cdebc8a94174e",
                0,
                "a6df6a0bb448992a301df4258e06a89fe7cf7146f59ac3bd5ff26083acb22ceb",
            ),
            (
                "3a286147b25e16ae80aff406f2673c6e565418c40f45c071245cdebc8a94174e",
                1,
                "592095f44bb766d5cfe20bda71f9575ed2df6b9fb9addc7e5fdffe0923841456",
            ),
            (
                "3a286147b25e16ae80aff406f2673c6e565418c40f45c071245cdebc8a94174e",
                2,
                "a6df6a0bb448992a301df4258e06a89fe7cf7146f59ac3bd5ff26083acb22ceb",
            ),
        ],
        address: "sp1qqgste7k9hx0qftg6qmwlkqtwuy6cycyavzmzj85c6qdfhjdpdjtdgqjuexzk6murw56suy3e0rd2cgqvycxttddwsvgxe2usfpxumr70xc9pkqwv",
        output: "7e88a7536c90770be4d2693a84ed03abe3fdcc5a29f96ec3433effec3b0c2194",
    },
];

fn secret(byte: u8) -> SecretKey {
    SecretKey::from_slice(&[byte; 32]).unwrap()
}

fn public(secret: &SecretKey) -> PublicKey {
    PublicKey::from_secret_key(&Secp256k1::new(), secret)
}

/// `secret·point`, as the sender multiplies a recipient key by an input secret.
fn multiply(point: &PublicKey, secret: &SecretKey) -> PublicKey {
    point
        .mul_tweak(&Secp256k1::new(), &Scalar::from(*secret))
        .unwrap()
}

fn spending(outpoints: &[OutPoint]) -> Transaction {
    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: outpoints
            .iter()
            .map(|outpoint| TxIn {
                previous_output: *outpoint,
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            })
            .collect(),
        output: Vec::new(),
    }
}

#[test_all]
fn test_bip352_output_keys() {
    for vector in VECTORS {
        let address: SilentPaymentAddress = vector.address.parse().unwrap();
        let secrets: Vec<SecretKey> = vector
            .inputs
            .iter()
            .map(|(_, _, secret)| SecretKey::from_str(secret).unwrap())
            .collect();
        let outpoints: Vec<OutPoint> = vector
            .inputs
            .iter()
            .map(|(txid, vout, _)| OutPoint::new(Txid::from_str(txid).unwrap(), *vout))
            .collect();

        // The sums go over public points, so an intermediate sum may be zero.
        let input_keys: Vec<PublicKey> = secrets.iter().map(public).collect();
        let shares: Vec<PublicKey> = secrets
            .iter()
            .map(|secret| multiply(&address.scan_key, secret))
            .collect();
        let input_key_sum =
            PublicKey::combine_keys(&input_keys.iter().collect::<Vec<_>>()).unwrap();
        let ecdh_share = PublicKey::combine_keys(&shares.iter().collect::<Vec<_>>()).unwrap();

        let input_hash = input_hash(&spending(&outpoints), &input_key_sum).unwrap();
        let output_key = output_key(&address.spend_key, &ecdh_share, &input_hash).unwrap();
        assert_eq!(output_key.to_string(), vector.output, "{}", vector.comment);
    }
}

struct Fixture {
    exit_tx: Transaction,
    address: SilentPaymentAddress,
    proof: SilentPaymentProof,
}

const PAYOUT_SATS: u64 = 15_000;

fn parent(script_pubkey: ScriptBuf, value: u64) -> Transaction {
    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::new(Txid::all_zeros(), value.try_into().unwrap()),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(value),
            script_pubkey,
        }],
    }
}

fn p2wpkh(key: &PublicKey) -> ScriptBuf {
    ScriptBuf::new_p2wpkh(&CompressedPublicKey(*key).wpubkey_hash())
}

/// A cooperative exit to a regtest silent payment address, funded by one
/// P2WPKH output per secret. `first_script` replaces the script of the first
/// input's spent output.
fn fixture_with(input_secrets: &[SecretKey], first_script: Option<ScriptBuf>) -> Fixture {
    let scan_secret = secret(1);
    let spend_secret = secret(2);
    let address = SilentPaymentAddress::new(
        public(&scan_secret),
        public(&spend_secret),
        Network::Regtest,
    );

    let input_keys: Vec<PublicKey> = input_secrets.iter().map(public).collect();
    let mut scripts: Vec<ScriptBuf> = input_keys.iter().map(p2wpkh).collect();
    if let (Some(first_script), Some(script)) = (first_script, scripts.first_mut()) {
        *script = first_script;
    }
    let parents: Vec<Transaction> = scripts
        .iter()
        .zip(100_000u64..)
        .map(|(script, value)| parent(script.clone(), value))
        .collect();
    let outpoints: Vec<OutPoint> = parents
        .iter()
        .map(|parent| OutPoint::new(parent.compute_txid(), 0))
        .collect();
    let mut exit_tx = spending(&outpoints);

    let input_secret_sum = input_secrets
        .iter()
        .skip(1)
        .try_fold(input_secrets[0], |sum, secret| {
            sum.add_tweak(&Scalar::from(*secret)).ok()
        });
    let (ecdh_share, dleq_proof, payout_script) = match input_secret_sum {
        Some(sum) => {
            let ecdh_share = multiply(&address.scan_key, &sum);
            let input_hash = input_hash(&exit_tx, &public(&sum)).unwrap();
            let output_key = output_key(&address.spend_key, &ecdh_share, &input_hash).unwrap();
            let payout_script =
                ScriptBuf::new_p2tr_tweaked(TweakedPublicKey::dangerous_assume_tweaked(output_key));
            (
                ecdh_share,
                generate_proof(&sum, &address.scan_key),
                payout_script,
            )
        }
        // The input secrets cancel out, so no payout key exists.
        None => (address.scan_key, [0u8; 64], ScriptBuf::new()),
    };
    exit_tx.output = vec![
        TxOut {
            value: Amount::from_sat(PAYOUT_SATS),
            script_pubkey: payout_script,
        },
        TxOut {
            value: Amount::from_sat(5_280),
            script_pubkey: p2wpkh(&public(&secret(30))),
        },
    ];

    let proof = SilentPaymentProof {
        output_index: 0,
        input_public_keys: input_keys,
        input_outpoints: outpoints,
        input_prevout_scripts: scripts,
        input_prevout_transactions: parents,
        ecdh_share,
        dleq_proof,
    };
    Fixture {
        exit_tx,
        address,
        proof,
    }
}

fn fixture() -> Fixture {
    fixture_with(&[secret(11), secret(12)], None)
}

fn validate(fixture: &Fixture, expected_payout_sats: u64) -> Result<(), SilentPaymentProofError> {
    validate_silent_payment_payout(
        &fixture.exit_tx,
        &fixture.address,
        &fixture.proof,
        expected_payout_sats,
    )
}

#[test_all]
fn test_valid_payout() {
    let fixture = fixture();
    assert_eq!(validate(&fixture, PAYOUT_SATS), Ok(()));
}

#[test_all]
fn test_recipient_finds_payout() {
    let fixture = fixture();
    let scan_secret = secret(1);
    let input_keys: Vec<&PublicKey> = fixture.proof.input_public_keys.iter().collect();
    let input_key_sum = PublicKey::combine_keys(&input_keys).unwrap();
    let recipient_share = multiply(&input_key_sum, &scan_secret);
    assert_eq!(recipient_share, fixture.proof.ecdh_share);

    let input_hash = input_hash(&fixture.exit_tx, &input_key_sum).unwrap();
    let output_key = output_key(&fixture.address.spend_key, &recipient_share, &input_hash).unwrap();
    assert_eq!(
        fixture.exit_tx.output[0].script_pubkey,
        ScriptBuf::new_p2tr_tweaked(TweakedPublicKey::dangerous_assume_tweaked(output_key))
    );
}

#[test_all]
fn test_input_count() {
    let mut fixture = fixture();
    fixture.proof.input_prevout_scripts.pop();
    assert_eq!(
        validate(&fixture, PAYOUT_SATS),
        Err(SilentPaymentProofError::InputCount {
            listed: 1,
            inputs: 2
        })
    );
}

#[test_all]
fn test_signed_input() {
    let mut fixture = fixture();
    fixture.exit_tx.input[1].witness.push([1]);
    assert_eq!(
        validate(&fixture, PAYOUT_SATS),
        Err(SilentPaymentProofError::SignedInput(1))
    );

    let mut fixture = self::fixture();
    fixture.exit_tx.input[0].script_sig = ScriptBuf::from_bytes(vec![0x51]);
    assert_eq!(
        validate(&fixture, PAYOUT_SATS),
        Err(SilentPaymentProofError::SignedInput(0))
    );
}

#[test_all]
fn test_duplicate_outpoint() {
    let mut fixture = fixture();
    let outpoint = fixture.exit_tx.input[0].previous_output;
    fixture.exit_tx.input[1].previous_output = outpoint;
    assert_eq!(
        validate(&fixture, PAYOUT_SATS),
        Err(SilentPaymentProofError::DuplicateOutpoint(outpoint))
    );
}

#[test_all]
fn test_outpoint_mismatch() {
    let mut fixture = fixture();
    fixture.proof.input_outpoints.swap(0, 1);
    assert_eq!(
        validate(&fixture, PAYOUT_SATS),
        Err(SilentPaymentProofError::OutpointMismatch {
            index: 0,
            listed: fixture.proof.input_outpoints[0],
            spent: fixture.exit_tx.input[0].previous_output,
        })
    );
}

#[test_all]
fn test_parent_mismatch() {
    let mut fixture = fixture();
    fixture.proof.input_prevout_transactions.swap(0, 1);
    assert_eq!(
        validate(&fixture, PAYOUT_SATS),
        Err(SilentPaymentProofError::ParentMismatch(0))
    );
}

#[test_all]
fn test_missing_prevout() {
    let mut fixture = fixture();
    fixture.exit_tx.input[0].previous_output.vout = 5;
    fixture.proof.input_outpoints[0].vout = 5;
    assert_eq!(
        validate(&fixture, PAYOUT_SATS),
        Err(SilentPaymentProofError::MissingPrevout { index: 0, vout: 5 })
    );
}

#[test_all]
fn test_prevout_script_mismatch() {
    let mut fixture = fixture();
    fixture.proof.input_prevout_scripts[1] = p2wpkh(&public(&secret(40)));
    assert_eq!(
        validate(&fixture, PAYOUT_SATS),
        Err(SilentPaymentProofError::PrevoutScriptMismatch(1))
    );
}

#[test_all]
fn test_unsupported_inputs() {
    let key = public(&secret(11));
    let p2wsh = ScriptBuf::new_p2wsh(&WScriptHash::all_zeros());
    let p2tr = ScriptBuf::new_p2tr_tweaked(TweakedPublicKey::dangerous_assume_tweaked(
        key.x_only_public_key().0,
    ));
    let segwit_v2 = ScriptBuf::new_witness_program(
        &bitcoin::WitnessProgram::new(bitcoin::WitnessVersion::V2, &[7u8; 32]).unwrap(),
    );
    let p2sh = ScriptBuf::new_p2sh(&p2wpkh(&key).script_hash());
    let p2pkh = ScriptBuf::new_p2pkh(&bitcoin::PublicKey::new(key).pubkey_hash());
    let bare = ScriptBuf::new_p2pk(&bitcoin::PublicKey::new(key));

    for (script, kind) in [
        (p2wsh, InputKind::P2wsh),
        (p2tr, InputKind::P2tr),
        (segwit_v2, InputKind::OtherWitness),
        (p2sh, InputKind::P2sh),
        (p2pkh, InputKind::P2pkh),
        (bare, InputKind::Other),
    ] {
        let fixture = fixture_with(&[secret(11), secret(12)], Some(script));
        assert_eq!(
            validate(&fixture, PAYOUT_SATS),
            Err(SilentPaymentProofError::UnsupportedInput { index: 0, kind }),
            "{kind}"
        );
    }
}

#[test_all]
fn test_input_key_mismatch() {
    let mut fixture = fixture();
    fixture.proof.input_public_keys[0] = public(&secret(40));
    assert_eq!(
        validate(&fixture, PAYOUT_SATS),
        Err(SilentPaymentProofError::InputKeyMismatch(0))
    );
}

#[test_all]
fn test_input_keys_cancel_out() {
    let input = secret(11);
    let fixture = fixture_with(&[input, input.negate()], None);
    assert_eq!(
        validate(&fixture, PAYOUT_SATS),
        Err(SilentPaymentProofError::InputKeySum)
    );
}

#[test_all]
fn test_tampered_dleq_proof() {
    let mut fixture = fixture();
    fixture.proof.dleq_proof[63] ^= 1;
    assert_eq!(
        validate(&fixture, PAYOUT_SATS),
        Err(SilentPaymentProofError::InvalidDleqProof)
    );
}

#[test_all]
fn test_shared_point_from_another_secret() {
    // A share with a valid proof for another secret does not match the inputs.
    let mut fixture = fixture();
    let other = secret(50);
    fixture.proof.ecdh_share = multiply(&fixture.address.scan_key, &other);
    fixture.proof.dleq_proof = generate_proof(&other, &fixture.address.scan_key);
    assert_eq!(
        validate(&fixture, PAYOUT_SATS),
        Err(SilentPaymentProofError::InvalidDleqProof)
    );
}

#[test_all]
fn test_payout_index() {
    let mut fixture = fixture();
    fixture.proof.output_index = 1;
    assert_eq!(
        validate(&fixture, PAYOUT_SATS),
        Err(SilentPaymentProofError::PayoutIndex(1))
    );
}

#[test_all]
fn test_payout_to_another_key() {
    let mut fixture = fixture();
    fixture.exit_tx.output[0].script_pubkey = p2wpkh(&public(&secret(60)));
    assert_eq!(
        validate(&fixture, PAYOUT_SATS),
        Err(SilentPaymentProofError::PayoutScript)
    );
}

#[test_all]
fn test_payout_amount() {
    let fixture = fixture();
    assert_eq!(
        validate(&fixture, PAYOUT_SATS + 1),
        Err(SilentPaymentProofError::PayoutAmount {
            paid: PAYOUT_SATS,
            expected: PAYOUT_SATS + 1
        })
    );
}
