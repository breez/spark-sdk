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

/// Default CPFP signer that handles P2WPKH and P2TR inputs using a single private key.
///
/// This signer detects the input type from the `witness_utxo` scriptPubKey:
/// - **P2WPKH** inputs are signed with ECDSA
/// - **P2TR** inputs are signed with Schnorr as a key-path spend with no script
///   tree (empty merkle root, BIP341 `tap_tweak` applied with `None`). Taproot
///   outputs that commit to a script tree are not supported by this signer;
///   callers with such outputs must implement `CpfpSigner` themselves.
/// - **Ephemeral anchor** inputs (already finalized) are skipped
#[cfg_attr(feature = "uniffi", derive(uniffi::Object))]
pub struct SingleKeySigner {
    secret_key: SecretKey,
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl SingleKeySigner {
    /// Create a new `SingleKeySigner` from a 32-byte secret key.
    #[cfg_attr(feature = "uniffi", uniffi::constructor)]
    #[allow(clippy::needless_pass_by_value)] // UniFFI requires owned Vec<u8>
    pub fn new(secret_key_bytes: Vec<u8>) -> Result<Self, SignerError> {
        let secret_key = SecretKey::from_slice(&secret_key_bytes)
            .map_err(|e| SignerError::InvalidInput(format!("Invalid secret key: {e}")))?;
        Ok(Self { secret_key })
    }
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[macros::async_trait]
impl CpfpSigner for SingleKeySigner {
    async fn sign_psbt(&self, psbt_bytes: Vec<u8>) -> Result<Vec<u8>, SignerError> {
        let mut psbt = bitcoin::Psbt::deserialize(&psbt_bytes)
            .map_err(|e| SignerError::InvalidInput(format!("Invalid PSBT: {e}")))?;

        let secp = Secp256k1::new();
        let pubkey = self.secret_key.public_key(&secp);
        let bitcoin_pubkey = bitcoin::PublicKey::new(pubkey);

        let prevouts: Vec<bitcoin::TxOut> = psbt
            .inputs
            .iter()
            .map(|input| input.witness_utxo.clone().unwrap_or(bitcoin::TxOut::NULL))
            .collect();

        let mut cache = SighashCache::new(&psbt.unsigned_tx);
        let mut ecdsa_signatures = vec![];
        let mut taproot_indices = vec![];

        for (i, input) in psbt.inputs.iter().enumerate() {
            // Skip inputs already finalized (e.g. the ephemeral anchor)
            if input.final_script_witness.is_some() {
                continue;
            }

            let Some(tx_out) = &input.witness_utxo else {
                continue;
            };

            if tx_out.script_pubkey.is_p2tr() {
                taproot_indices.push(i);
            } else {
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
            }
        }

        // Finalize ECDSA (P2WPKH) inputs
        for (i, pk, signature) in ecdsa_signatures {
            let mut witness = Witness::new();
            witness.push(signature.to_vec());
            witness.push(pk.to_bytes());
            psbt.inputs[i].final_script_witness = Some(witness);
            psbt.inputs[i].partial_sigs.clear();
        }

        // Sign and finalize taproot inputs
        if !taproot_indices.is_empty() {
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
