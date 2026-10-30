//! BIP-374 DLEQ proof verification: a proof that `A = a·G` and `C = a·B` share
//! the scalar `a`, without revealing it.

use bitcoin::secp256k1::{PublicKey, Scalar, Secp256k1, Verification, constants::CURVE_ORDER};

use super::tagged_hash;

const CHALLENGE_TAG: &str = "BIP0374/challenge";

pub(super) fn verify_proof(
    a: &PublicKey,
    b: &PublicKey,
    c: &PublicKey,
    proof: &[u8; 64],
    g: &PublicKey,
    message: Option<&[u8; 32]>,
) -> bool {
    let secp = Secp256k1::verification_only();
    let (e_bytes, s_bytes) = proof.split_at(32);
    let (Ok(e), Ok(s)) = (scalar(e_bytes), scalar(s_bytes)) else {
        return false;
    };
    let Some(r1) = difference(&secp, g, &s, a, &e) else {
        return false;
    };
    let Some(r2) = difference(&secp, b, &s, c, &e) else {
        return false;
    };
    let challenge = challenge(a, b, c, g, &r1, &r2, message);
    challenge.as_slice() == e_bytes
}

/// `int(bytes) mod n` per BIP-374. A hash is below `2n`, so one subtraction
/// of the order reduces it.
pub(super) fn reduce(bytes: [u8; 32]) -> [u8; 32] {
    if Scalar::from_be_bytes(bytes).is_ok() {
        return bytes;
    }
    let mut reduced = [0u8; 32];
    let mut borrow = false;
    for ((out, byte), order) in reduced
        .iter_mut()
        .zip(bytes.iter())
        .zip(CURVE_ORDER.iter())
        .rev()
    {
        let (difference, below_order) = byte.overflowing_sub(*order);
        let (difference, below_borrow) = difference.overflowing_sub(u8::from(borrow));
        *out = difference;
        borrow = below_order || below_borrow;
    }
    reduced
}

pub(super) fn challenge(
    a: &PublicKey,
    b: &PublicKey,
    c: &PublicKey,
    g: &PublicKey,
    r1: &PublicKey,
    r2: &PublicKey,
    message: Option<&[u8; 32]>,
) -> [u8; 32] {
    let message: &[u8] = message.map_or(&[], |message| message);
    reduce(tagged_hash(
        CHALLENGE_TAG,
        &[
            &a.serialize(),
            &b.serialize(),
            &c.serialize(),
            &g.serialize(),
            &r1.serialize(),
            &r2.serialize(),
            message,
        ],
    ))
}

fn scalar(bytes: &[u8]) -> Result<Scalar, ()> {
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| ())?;
    Scalar::from_be_bytes(bytes).map_err(|_| ())
}

/// `s·p - e·q`, or `None` for the point at infinity.
fn difference<C: Verification>(
    secp: &Secp256k1<C>,
    p: &PublicKey,
    s: &Scalar,
    q: &PublicKey,
    e: &Scalar,
) -> Option<PublicKey> {
    // `mul_tweak` fails exactly when the scalar is zero, the one case where the
    // product is the point at infinity.
    let sp = p.mul_tweak(secp, s).ok();
    let negative_eq = q.mul_tweak(secp, e).ok().map(|point| point.negate(secp));
    match (sp, negative_eq) {
        (Some(sp), Some(negative_eq)) => sp.combine(&negative_eq).ok(),
        (Some(point), None) | (None, Some(point)) => Some(point),
        (None, None) => None,
    }
}

#[cfg(test)]
pub(super) mod tests {
    use std::str::FromStr;

    use bitcoin::secp256k1::SecretKey;
    use macros::test_all;

    use super::*;
    use crate::services::silent_payment::generator;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// BIP-374 `GenerateProof` with all-zero auxiliary data and the standard
    /// generator, for tests that need a valid proof.
    pub(crate) fn generate_proof(a: &SecretKey, b: &PublicKey) -> [u8; 64] {
        let secp = Secp256k1::new();
        let big_a = PublicKey::from_secret_key(&secp, a);
        let c = b.mul_tweak(&secp, &Scalar::from(*a)).unwrap();
        let aux = tagged_hash("BIP0374/aux", &[&[0u8; 32]]);
        let t: Vec<u8> = a
            .secret_bytes()
            .iter()
            .zip(aux)
            .map(|(x, y)| x ^ y)
            .collect();
        let rand = tagged_hash("BIP0374/nonce", &[&t, &big_a.serialize(), &c.serialize()]);
        let k = SecretKey::from_slice(&reduce(rand)).unwrap();
        let r1 = PublicKey::from_secret_key(&secp, &k);
        let r2 = b.mul_tweak(&secp, &Scalar::from(k)).unwrap();
        let e = challenge(&big_a, b, &c, &generator(), &r1, &r2, None);
        let s = a
            .mul_tweak(&Scalar::from_be_bytes(e).unwrap())
            .unwrap()
            .add_tweak(&Scalar::from(k))
            .unwrap();
        let mut proof = [0u8; 64];
        proof[..32].copy_from_slice(&e);
        proof[32..].copy_from_slice(&s.secret_bytes());
        proof
    }

    #[test_all]
    fn test_bip374_verify_vectors() {
        let vectors = include_str!("../../../testdata/bip374_test_vectors_verify_proof.csv");
        for line in vectors.lines().skip(1) {
            let fields: Vec<&str> = line.split(',').collect();
            let point = |i: usize| PublicKey::from_str(fields[i]);
            let (Ok(g), Ok(a), Ok(b), Ok(c)) = (point(1), point(2), point(3), point(4)) else {
                // The vectors contain only valid points.
                panic!("vector {} has an invalid point", fields[0]);
            };
            let proof: [u8; 64] = hex::decode(fields[5]).unwrap().try_into().unwrap();
            let message: Option<[u8; 32]> = (!fields[6].is_empty())
                .then(|| hex::decode(fields[6]).unwrap().try_into().unwrap());
            let expected = fields[7] == "TRUE";
            assert_eq!(
                verify_proof(&a, &b, &c, &proof, &g, message.as_ref()),
                expected,
                "vector {}: {}",
                fields[0],
                fields[8]
            );
        }
    }

    #[test_all]
    fn test_generated_proof_verifies() {
        let secp = Secp256k1::new();
        let a = secret(3);
        let b = PublicKey::from_secret_key(&secp, &secret(4));
        let proof = generate_proof(&a, &b);
        let big_a = PublicKey::from_secret_key(&secp, &a);
        let c = b.mul_tweak(&secp, &Scalar::from(a)).unwrap();
        assert!(verify_proof(&big_a, &b, &c, &proof, &generator(), None));

        let other_c = b.mul_tweak(&secp, &Scalar::from(secret(5))).unwrap();
        assert!(!verify_proof(
            &big_a,
            &b,
            &other_c,
            &proof,
            &generator(),
            None
        ));
    }

    #[test_all]
    fn test_reduce() {
        let mut above_order = CURVE_ORDER;
        above_order[31] = above_order[31].wrapping_add(5);
        let mut five = [0u8; 32];
        five[31] = 5;
        assert_eq!(reduce(above_order), five);
        assert_eq!(reduce(five), five);
        assert_eq!(reduce(CURVE_ORDER), [0u8; 32]);
    }

    fn secret(byte: u8) -> SecretKey {
        SecretKey::from_slice(&[byte; 32]).unwrap()
    }
}
