use bitcoin::{
    Address, Network, TapNodeHash, TapSighash, XOnlyPublicKey,
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
    tx: &bitcoin::Transaction,
    input_index: usize,
    prev_output: &bitcoin::TxOut,
) -> Result<TapSighash, BitcoinError> {
    let prevouts_arr = [prev_output.clone()];
    let prev_output_fetcher = bitcoin::sighash::Prevouts::All(&prevouts_arr);

    Ok(
        bitcoin::sighash::SighashCache::new(tx).taproot_key_spend_signature_hash(
            input_index,
            &prev_output_fetcher,
            bitcoin::sighash::TapSighashType::Default,
        )?,
    )
}
