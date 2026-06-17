//! Shared Turnkey wallet-account operations and byte conversions used by both
//! the Spark signer and the SDK-layer (Breez) signer.

use bitcoin::NetworkKind;
use bitcoin::bip32::{ChainCode, ChildNumber, Fingerprint, Xpriv};
use bitcoin::secp256k1::{SecretKey, ecdsa, schnorr};

use crate::Network;

use turnkey_enclave_encrypt::{ExportClient, QuorumPublicKey};

use super::error::TurnkeyError;
use super::transport::{OnConflict, TurnkeyClient};
use super::types::{
    ADDRESS_FORMAT_COMPRESSED, ADDRESS_FORMAT_SPARK_MAINNET, ADDRESS_FORMAT_SPARK_REGTEST,
    CREATE_WALLET_ACCOUNTS_PATH, CREATE_WALLET_ACCOUNTS_RESULT, CREATE_WALLET_ACCOUNTS_TYPE,
    CURVE_SECP256K1, CreateWalletAccountsIntent, CreateWalletAccountsResult, ENCODING_HEXADECIMAL,
    EXPORT_WALLET_ACCOUNT_PATH, EXPORT_WALLET_ACCOUNT_RESULT, EXPORT_WALLET_ACCOUNT_TYPE,
    ExportWalletAccountIntent, ExportWalletAccountResult, GET_WALLET_ACCOUNT_PATH,
    GetWalletAccountRequest, GetWalletAccountResponse, PATH_FORMAT_BIP32, SIGN_RAW_PAYLOAD_PATH,
    SIGN_RAW_PAYLOAD_RESULT, SIGN_RAW_PAYLOAD_TYPE, SignRawPayloadIntent, SignRawPayloadResult,
    WalletAccountParams,
};

/// The Spark address format (and thus the BIP-340 Schnorr signing scheme) for
/// the wallet's network.
pub(crate) fn spark_address_format(network: Network) -> &'static str {
    match network {
        Network::Mainnet => ADDRESS_FORMAT_SPARK_MAINNET,
        Network::Regtest => ADDRESS_FORMAT_SPARK_REGTEST,
    }
}

impl TurnkeyClient {
    /// Materializes the wallet account at `path` with `address_format`,
    /// returning its address. Idempotent: a path can only be created once, so if
    /// it already exists (a prior run or a concurrent request) Turnkey 409s and
    /// we read the existing address back. The `address_format` therefore only
    /// takes effect on first creation.
    pub(crate) async fn create_account(
        &self,
        path: String,
        address_format: &'static str,
    ) -> Result<String, TurnkeyError> {
        let intent = CreateWalletAccountsIntent {
            wallet_id: self.wallet_id.clone(),
            accounts: vec![WalletAccountParams {
                curve: CURVE_SECP256K1,
                path_format: PATH_FORMAT_BIP32,
                path: path.clone(),
                address_format,
            }],
        };
        let result: Result<CreateWalletAccountsResult, TurnkeyError> = self
            .submit_activity(
                CREATE_WALLET_ACCOUNTS_PATH,
                CREATE_WALLET_ACCOUNTS_TYPE,
                intent,
                CREATE_WALLET_ACCOUNTS_RESULT,
                // A 409 here is a permanent "account at this path exists", which
                // we recover from by reading the existing address.
                OnConflict::Surface,
            )
            .await;
        match result {
            Ok(result) => result.addresses.into_iter().next().ok_or_else(|| {
                TurnkeyError::UnexpectedResponse(
                    "create_wallet_accounts returned no address".into(),
                )
            }),
            Err(TurnkeyError::Http { status: 409, .. }) => self.account_address(path).await,
            Err(e) => Err(e),
        }
    }

    /// Reads the address of the existing wallet account at `path`.
    async fn account_address(&self, path: String) -> Result<String, TurnkeyError> {
        let request = GetWalletAccountRequest {
            organization_id: self.organization_id.clone(),
            wallet_id: self.wallet_id.clone(),
            path,
        };
        let resp: GetWalletAccountResponse = self
            .process_request(GET_WALLET_ACCOUNT_PATH, &request, OnConflict::Surface)
            .await?;
        Ok(resp.account.address)
    }

    /// Compressed public key (hex) for `path`: prefers an existing account's
    /// `publicKey`, else materializes an `ADDRESS_FORMAT_COMPRESSED` account
    /// (whose address is the compressed key).
    pub(crate) async fn compressed_pubkey_at(&self, path: String) -> Result<String, TurnkeyError> {
        let request = GetWalletAccountRequest {
            organization_id: self.organization_id.clone(),
            wallet_id: self.wallet_id.clone(),
            path: path.clone(),
        };
        if let Ok(resp) = self
            .process_request::<_, GetWalletAccountResponse>(
                GET_WALLET_ACCOUNT_PATH,
                &request,
                OnConflict::Surface,
            )
            .await
            && let Some(public_key) = resp.account.public_key
        {
            return Ok(public_key);
        }
        self.create_account(path, ADDRESS_FORMAT_COMPRESSED).await
    }

    /// Submits a `SIGN_RAW_PAYLOAD` activity: signs `payload_hex` with the
    /// `sign_with` account using `hash_function`.
    pub(crate) async fn sign_raw(
        &self,
        sign_with: String,
        payload_hex: String,
        hash_function: &'static str,
    ) -> Result<SignRawPayloadResult, TurnkeyError> {
        self.submit_activity(
            SIGN_RAW_PAYLOAD_PATH,
            SIGN_RAW_PAYLOAD_TYPE,
            SignRawPayloadIntent {
                sign_with,
                payload: payload_hex,
                encoding: ENCODING_HEXADECIMAL,
                hash_function,
            },
            SIGN_RAW_PAYLOAD_RESULT,
            OnConflict::Retry,
        )
        .await
    }

    /// Exports the secret key for the account at `path`, decrypting the bundle
    /// against the pinned production quorum key. Used where Turnkey's design
    /// requires a local key (static-deposit refund, SDK-layer encryption/HMAC).
    pub(crate) async fn export_secret_key(
        &self,
        path: String,
        address_format: &'static str,
    ) -> Result<SecretKey, TurnkeyError> {
        let address = self.create_account(path, address_format).await?;
        let mut export_client = ExportClient::new(&QuorumPublicKey::production_signer());
        let target_public_key = export_client
            .target_public_key()
            .map_err(|e| TurnkeyError::Deserialize(format!("export target key: {e}")))?;
        let result: ExportWalletAccountResult = self
            .submit_activity(
                EXPORT_WALLET_ACCOUNT_PATH,
                EXPORT_WALLET_ACCOUNT_TYPE,
                ExportWalletAccountIntent {
                    address,
                    target_public_key,
                },
                EXPORT_WALLET_ACCOUNT_RESULT,
                OnConflict::Retry,
            )
            .await?;
        let private_bytes = export_client
            .decrypt_private_key(&result.export_bundle, &self.organization_id)
            .map_err(|e| TurnkeyError::Deserialize(format!("export decrypt: {e}")))?;
        SecretKey::from_slice(&private_bytes)
            .map_err(|e| TurnkeyError::Deserialize(format!("export secret key: {e}")))
    }
}

/// Decodes a hex scalar into 32 bytes, left-padding if Turnkey omitted leading
/// zeros.
pub(crate) fn decode_scalar_32(hex_str: &str) -> Result<[u8; 32], TurnkeyError> {
    let bytes =
        hex::decode(hex_str).map_err(|e| TurnkeyError::Deserialize(format!("scalar hex: {e}")))?;
    if bytes.len() > 32 {
        return Err(TurnkeyError::Deserialize("scalar exceeds 32 bytes".into()));
    }
    let mut out = [0u8; 32];
    let start = 32usize.saturating_sub(bytes.len());
    out[start..].copy_from_slice(&bytes);
    Ok(out)
}

pub(crate) fn ecdsa_from_rs(r_hex: &str, s_hex: &str) -> Result<ecdsa::Signature, TurnkeyError> {
    let mut compact = [0u8; 64];
    compact[..32].copy_from_slice(&decode_scalar_32(r_hex)?);
    compact[32..].copy_from_slice(&decode_scalar_32(s_hex)?);
    let mut signature = ecdsa::Signature::from_compact(&compact)
        .map_err(|e| TurnkeyError::Deserialize(e.to_string()))?;
    signature.normalize_s();
    Ok(signature)
}

/// Normalizes a recoverable ECDSA signature to low-s, returning the 64-byte
/// compact `r || s` and the matching recovery id. Turnkey returns the raw `s`,
/// which may be high; secp256k1 verifiers and BIP-146 require low-s. Negating
/// `s` (s -> n - s) negates the nonce point R, flipping its y-parity, so the
/// recovery id's low bit must flip with it or the signature recovers to the
/// wrong key.
pub(crate) fn ecdsa_recoverable_low_s(
    r_hex: &str,
    s_hex: &str,
    recovery_id: u8,
) -> Result<([u8; 64], u8), TurnkeyError> {
    let mut compact = [0u8; 64];
    compact[..32].copy_from_slice(&decode_scalar_32(r_hex)?);
    compact[32..].copy_from_slice(&decode_scalar_32(s_hex)?);
    let mut signature = ecdsa::Signature::from_compact(&compact)
        .map_err(|e| TurnkeyError::Deserialize(e.to_string()))?;
    let before = signature.serialize_compact();
    signature.normalize_s();
    let after = signature.serialize_compact();
    let recovery_id = if before == after {
        recovery_id
    } else {
        recovery_id ^ 1
    };
    Ok((after, recovery_id))
}

pub(crate) fn schnorr_from_rs(
    r_hex: &str,
    s_hex: &str,
) -> Result<schnorr::Signature, TurnkeyError> {
    let mut sig = [0u8; 64];
    sig[..32].copy_from_slice(&decode_scalar_32(r_hex)?);
    sig[32..].copy_from_slice(&decode_scalar_32(s_hex)?);
    schnorr::Signature::from_slice(&sig).map_err(|e| TurnkeyError::Deserialize(e.to_string()))
}

/// Wraps a raw exported secret key as an `Xpriv` so it can root a local signer.
/// The chain code is a fixed placeholder: the master key is used directly via
/// the empty path, and any child derivations off it are deterministic (the
/// exported key plus this fixed chain code), which is all the consistency the
/// ECIES/HMAC and static-deposit-refund uses require.
pub(crate) fn xpriv_from_secret(secret: SecretKey, network: Network) -> Xpriv {
    Xpriv {
        network: match network {
            Network::Mainnet => NetworkKind::Main,
            Network::Regtest => NetworkKind::Test,
        },
        depth: 0,
        parent_fingerprint: Fingerprint::default(),
        child_number: ChildNumber::from_normal_idx(0).expect("0 is a valid child index"),
        private_key: secret,
        chain_code: ChainCode::from([0u8; 32]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::secp256k1::ecdsa::{RecoverableSignature, RecoveryId};
    use bitcoin::secp256k1::{Message, PublicKey, Secp256k1, SecretKey};

    // secp256k1 group order, big-endian.
    const N: [u8; 32] = [
        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        0xFE, 0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C, 0xD0, 0x36,
        0x41, 0x41,
    ];

    /// Big-endian `N - s`, used to build the high-s twin of a low-s scalar.
    fn neg_scalar(s: &[u8; 32]) -> [u8; 32] {
        let mut out = [0u8; 32];
        let mut borrow = false;
        for i in (0..32).rev() {
            let (d1, b1) = N[i].overflowing_sub(s[i]);
            let (d2, b2) = d1.overflowing_sub(u8::from(borrow));
            out[i] = d2;
            borrow = b1 || b2;
        }
        out
    }

    fn recovered_key(compact: &[u8; 64], recovery_id: u8, msg: &Message) -> PublicKey {
        let secp = Secp256k1::new();
        let recid = RecoveryId::from_i32(i32::from(recovery_id)).unwrap();
        let sig = RecoverableSignature::from_compact(compact, recid).unwrap();
        secp.recover_ecdsa(msg, &sig).unwrap()
    }

    // A high-s signature from Turnkey must normalize to low-s without losing
    // recoverability: negating `s` flips the recovery id's low bit, and both the
    // raw and normalized forms must recover the same key.
    #[test]
    fn low_s_normalization_preserves_recovery() {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let pk = PublicKey::from_secret_key(&secp, &sk);
        let msg = Message::from_digest([0x42; 32]);

        // secp256k1 always signs low-s, so this is the canonical form.
        let (recid_low, compact_low) = secp.sign_ecdsa_recoverable(&msg, &sk).serialize_compact();
        let recid_low = u8::try_from(recid_low.to_i32()).unwrap();
        let r_hex = hex::encode(&compact_low[..32]);
        let s_low_hex = hex::encode(&compact_low[32..]);

        // Low-s input passes through untouched and recovers `pk`.
        let (out, recid) = ecdsa_recoverable_low_s(&r_hex, &s_low_hex, recid_low).unwrap();
        assert_eq!(out, compact_low);
        assert_eq!(recid, recid_low);
        assert_eq!(recovered_key(&out, recid, &msg), pk);

        // High-s twin: s -> N - s, recovery id's low bit flipped. It recovers
        // `pk` as given, and normalizing it returns the original low-s form.
        let mut s_low = [0u8; 32];
        s_low.copy_from_slice(&compact_low[32..]);
        let s_high = neg_scalar(&s_low);
        let s_high_hex = hex::encode(s_high);
        let recid_high = recid_low ^ 1;

        let mut compact_high = compact_low;
        compact_high[32..].copy_from_slice(&s_high);
        assert_eq!(recovered_key(&compact_high, recid_high, &msg), pk);

        let (out, recid) = ecdsa_recoverable_low_s(&r_hex, &s_high_hex, recid_high).unwrap();
        assert_eq!(out, compact_low);
        assert_eq!(recid, recid_low);
        assert_eq!(recovered_key(&out, recid, &msg), pk);
    }
}
