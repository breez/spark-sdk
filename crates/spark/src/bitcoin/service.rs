use bitcoin::{
    Address, Network, TapNodeHash, TapSighash, Transaction, TxOut, XOnlyPublicKey,
    key::{Secp256k1, TapTweak, UntweakedPublicKey},
    secp256k1::{All, Message, PublicKey, ecdsa, schnorr},
};

use crate::bitcoin::BitcoinError;

pub struct BitcoinService {
    network: Network,
    secp: Secp256k1<All>,
}

impl BitcoinService {
    pub fn new(network: impl Into<Network>) -> Self {
        BitcoinService {
            network: network.into(),
            secp: Secp256k1::new(),
        }
    }

    pub fn compute_taproot_key_no_script(&self, pubkey: &PublicKey) -> XOnlyPublicKey {
        let (x_only_pub, _) = pubkey.x_only_public_key();

        // BIP341 taproot tweak with empty script tree
        let (tweaked_key, _parity) = x_only_pub.tap_tweak(&self.secp, None);

        tweaked_key.to_x_only_public_key()
    }

    pub fn is_valid_ecdsa_signature(
        &self,
        signature: &ecdsa::Signature,
        message: &Message,
        pubkey: &PublicKey,
    ) -> bool {
        self.secp.verify_ecdsa(message, signature, pubkey).is_ok()
    }

    pub fn is_valid_schnorr_signature(
        &self,
        sig: &schnorr::Signature,
        msg: &Message,
        pubkey: &XOnlyPublicKey,
    ) -> bool {
        self.secp.verify_schnorr(sig, msg, pubkey).is_ok()
    }

    pub fn p2tr_address(
        &self,
        internal_key: UntweakedPublicKey,
        merkle_root: Option<TapNodeHash>,
    ) -> Address {
        Address::p2tr(&self.secp, internal_key, merkle_root, self.network)
    }

    pub fn subtract_public_keys(
        &self,
        a: &PublicKey,
        b: &PublicKey,
    ) -> Result<PublicKey, BitcoinError> {
        let negated = b.negate(&self.secp);
        a.combine(&negated)
            .map_err(|e| BitcoinError::KeyCombinationError(e.to_string()))
    }
}

pub fn sighash_from_tx(
    tx: &Transaction,
    input_index: usize,
    prev_output: &TxOut,
) -> Result<TapSighash, BitcoinError> {
    // Fill for each input with the previous output
    let prevouts = vec![prev_output.clone(); tx.input.len()];
    let prev_output_fetcher = bitcoin::sighash::Prevouts::All(&prevouts);

    Ok(
        bitcoin::sighash::SighashCache::new(tx).taproot_key_spend_signature_hash(
            input_index,
            &prev_output_fetcher,
            bitcoin::sighash::TapSighashType::Default,
        )?,
    )
}

/// Computes the BIP 341 Taproot sighash for a transaction with multiple distinct inputs.
///
/// Unlike `sighash_from_tx` which duplicates a single prevout for all inputs, this function
/// takes an array of all previous outputs. This is required for transactions like coop exit
/// refunds that spend from multiple different UTXOs (node tx + connector tx).
///
/// # Arguments
///
/// * `tx` - The transaction to compute the sighash for
/// * `input_index` - The index of the input being signed
/// * `prev_outputs` - Slice containing the previous output for each input (must match input count)
///
/// # Errors
///
/// Returns `BitcoinError::InvalidTransaction` if the number of prevouts doesn't match the
/// number of inputs.
pub fn sighash_from_multi_input_tx(
    tx: &Transaction,
    input_index: usize,
    prev_outputs: &[TxOut],
) -> Result<TapSighash, BitcoinError> {
    if prev_outputs.len() != tx.input.len() {
        return Err(BitcoinError::InvalidTransaction(format!(
            "prev_outputs length {} != inputs length {}",
            prev_outputs.len(),
            tx.input.len()
        )));
    }
    let prev_output_fetcher = bitcoin::sighash::Prevouts::All(prev_outputs);
    Ok(
        bitcoin::sighash::SighashCache::new(tx).taproot_key_spend_signature_hash(
            input_index,
            &prev_output_fetcher,
            bitcoin::sighash::TapSighashType::Default,
        )?,
    )
}
