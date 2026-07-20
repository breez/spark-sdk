use std::sync::Arc;

use bitcoin::{
    Witness,
    ecdsa::Signature,
    hashes::Hash as _,
    key::{Secp256k1, TapTweak as _},
    secp256k1::SecretKey,
    sighash::{self, SighashCache},
};

use crate::error::SignerError;

use super::cpfp::CpfpSigner;

/// A CPFP signer backed by a single private key. Signs P2WPKH and P2TR key-path
/// inputs only; taproot script-path spends are not supported.
#[cfg_attr(feature = "uniffi", uniffi::export)]
#[allow(clippy::needless_pass_by_value)]
pub fn single_key_cpfp_signer(
    secret_key_bytes: Vec<u8>,
) -> Result<Arc<dyn CpfpSigner>, SignerError> {
    Ok(Arc::new(SingleKeySigner::new(secret_key_bytes)?))
}

pub struct SingleKeySigner {
    secret_key: SecretKey,
}

impl SingleKeySigner {
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(secret_key_bytes: Vec<u8>) -> Result<Self, SignerError> {
        let secret_key = SecretKey::from_slice(&secret_key_bytes)
            .map_err(|e| SignerError::InvalidInput(format!("Invalid secret key: {e}")))?;
        Ok(Self { secret_key })
    }
}

#[macros::async_trait]
impl CpfpSigner for SingleKeySigner {
    async fn sign_psbt(&self, psbt_bytes: Vec<u8>) -> Result<Vec<u8>, SignerError> {
        let mut psbt = bitcoin::Psbt::deserialize(&psbt_bytes)
            .map_err(|e| SignerError::InvalidInput(format!("Invalid PSBT: {e}")))?;

        let secp = Secp256k1::new();
        let pubkey = self.secret_key.public_key(&secp);
        let bitcoin_pubkey = bitcoin::PublicKey::new(pubkey);

        let mut prevouts: Vec<bitcoin::TxOut> = Vec::with_capacity(psbt.inputs.len());
        let mut has_placeholder_prevout = false;
        for input in &psbt.inputs {
            match (&input.witness_utxo, &input.final_script_witness) {
                (Some(tx_out), _) => prevouts.push(tx_out.clone()),
                // Not ours to sign; NULL is safe because taproot signing is
                // refused below whenever a prevout is a placeholder.
                (None, Some(_)) => {
                    has_placeholder_prevout = true;
                    prevouts.push(bitcoin::TxOut::NULL);
                }
                (None, None) => {
                    return Err(SignerError::InvalidInput(
                        "PSBT input is missing witness_utxo".to_string(),
                    ));
                }
            }
        }

        let mut cache = SighashCache::new(&psbt.unsigned_tx);
        let mut ecdsa_signatures = vec![];
        let mut taproot_indices = vec![];

        for (i, input) in psbt.inputs.iter().enumerate() {
            if input.final_script_witness.is_some() {
                continue;
            }
            let Some(tx_out) = &input.witness_utxo else {
                return Err(SignerError::InvalidInput(
                    "PSBT input is missing witness_utxo".to_string(),
                ));
            };

            if tx_out.script_pubkey.is_p2tr() {
                taproot_indices.push(i);
            } else if tx_out.script_pubkey.is_p2wpkh() {
                let (msg, ecdsa_type) = psbt
                    .sighash_ecdsa(i, &mut cache)
                    .map_err(|e| SignerError::Signing(format!("ECDSA sighash error: {e}")))?;
                let sig = secp.sign_ecdsa(&msg, &self.secret_key);
                ecdsa_signatures.push((
                    i,
                    bitcoin_pubkey,
                    Signature {
                        signature: sig,
                        sighash_type: ecdsa_type,
                    },
                ));
            } else {
                return Err(SignerError::InvalidInput(
                    "unsupported script type for the built-in single-key signer \
                     (only P2WPKH and P2TR key-path)"
                        .to_string(),
                ));
            }
        }

        for (i, pk, signature) in ecdsa_signatures {
            let mut witness = Witness::new();
            witness.push(signature.to_vec());
            witness.push(pk.to_bytes());
            psbt.inputs[i].final_script_witness = Some(witness);
            psbt.inputs[i].partial_sigs.clear();
        }

        if !taproot_indices.is_empty() {
            if has_placeholder_prevout {
                return Err(SignerError::InvalidInput(
                    "cannot sign taproot input: another PSBT input is missing witness_utxo"
                        .to_string(),
                ));
            }
            let keypair = bitcoin::key::Keypair::from_secret_key(&secp, &self.secret_key)
                .tap_tweak(&secp, None)
                .to_keypair();
            let prevouts_ref = sighash::Prevouts::All(&prevouts);
            for i in taproot_indices {
                let sighash = cache
                    .taproot_key_spend_signature_hash(
                        i,
                        &prevouts_ref,
                        sighash::TapSighashType::Default,
                    )
                    .map_err(|e| SignerError::Signing(format!("Taproot sighash error: {e}")))?;
                let msg = bitcoin::secp256k1::Message::from_digest(sighash.to_byte_array());
                let schnorr_sig = secp.sign_schnorr_no_aux_rand(&msg, &keypair);
                let tap_sig = bitcoin::taproot::Signature {
                    signature: schnorr_sig,
                    sighash_type: sighash::TapSighashType::Default,
                };
                let mut witness = Witness::new();
                witness.push(tap_sig.to_vec());
                psbt.inputs[i].final_script_witness = Some(witness);
                psbt.inputs[i].tap_key_sig = None;
            }
        }

        Ok(psbt.serialize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{
        Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness,
        absolute::LockTime, transaction::Version,
    };

    fn test_secret_key_bytes() -> Vec<u8> {
        vec![1u8; 32]
    }

    fn signer_p2wpkh_script() -> ScriptBuf {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&test_secret_key_bytes()).unwrap();
        let cpk = bitcoin::CompressedPublicKey(sk.public_key(&secp));
        ScriptBuf::new_p2wpkh(&cpk.wpubkey_hash())
    }

    fn dummy_txin(vout: u32) -> TxIn {
        TxIn {
            previous_output: OutPoint {
                txid: Txid::all_zeros(),
                vout,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }
    }

    fn empty_psbt(n: u32) -> bitcoin::Psbt {
        let tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: (0..n).map(dummy_txin).collect(),
            output: vec![TxOut {
                value: Amount::from_sat(1_000),
                script_pubkey: signer_p2wpkh_script(),
            }],
        };
        bitcoin::Psbt::from_unsigned_tx(tx).unwrap()
    }

    fn set_finalized_anchor(input: &mut bitcoin::psbt::Input) {
        input.witness_utxo = Some(TxOut {
            value: Amount::ZERO,
            script_pubkey: ScriptBuf::from(vec![0x51, 0x02, 0x4e, 0x73]),
        });
        input.final_script_witness = Some(Witness::new());
    }

    #[macros::async_test_all]
    async fn single_key_missing_witness_utxo_errors() {
        let psbt = empty_psbt(1);
        let signer = SingleKeySigner::new(test_secret_key_bytes()).unwrap();
        let res = signer.sign_psbt(psbt.serialize()).await;
        assert!(res.is_err(), "expected error for missing witness_utxo");
    }

    #[macros::async_test_all]
    async fn single_key_unsupported_script_type_errors() {
        let mut psbt = empty_psbt(1);
        let p2wsh = ScriptBuf::new_p2wsh(&bitcoin::WScriptHash::from_byte_array([2u8; 32]));
        psbt.inputs[0].witness_utxo = Some(TxOut {
            value: Amount::from_sat(5_000),
            script_pubkey: p2wsh,
        });
        let signer = SingleKeySigner::new(test_secret_key_bytes()).unwrap();
        let res = signer.sign_psbt(psbt.serialize()).await;
        assert!(res.is_err(), "expected error for unsupported script type");
    }

    #[macros::async_test_all]
    async fn single_key_signs_p2wpkh_and_skips_anchor() {
        let mut psbt = empty_psbt(2);
        psbt.inputs[0].witness_utxo = Some(TxOut {
            value: Amount::from_sat(10_000),
            script_pubkey: signer_p2wpkh_script(),
        });
        set_finalized_anchor(&mut psbt.inputs[1]);

        let signer = SingleKeySigner::new(test_secret_key_bytes()).unwrap();
        let signed_bytes = signer.sign_psbt(psbt.serialize()).await.unwrap();
        let out_psbt = bitcoin::Psbt::deserialize(&signed_bytes).unwrap();

        let funding_witness = out_psbt.inputs[0]
            .final_script_witness
            .as_ref()
            .expect("funding input should be finalized");
        assert_eq!(funding_witness.len(), 2, "P2WPKH witness is [sig, pubkey]");
        assert!(out_psbt.inputs[1].final_script_witness.is_some());
    }
}
