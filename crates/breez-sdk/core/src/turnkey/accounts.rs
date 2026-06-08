//! Shared Turnkey wallet-account operations and byte conversions used by both
//! the Spark signer and the SDK-layer (Breez) signer.

use bitcoin::NetworkKind;
use bitcoin::bip32::{ChainCode, ChildNumber, Fingerprint, Xpriv};
use bitcoin::secp256k1::{SecretKey, ecdsa, schnorr};

use crate::Network;

use turnkey_enclave_encrypt::{ExportClient, QuorumPublicKey};

use super::error::TurnkeyError;
use super::transport::TurnkeyClient;
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
    /// Materializes a wallet account at `path` with `address_format`, returning
    /// its address. The key is deterministic from the wallet seed, so this is
    /// effectively idempotent across runs.
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
                path,
                address_format,
            }],
        };
        let result: CreateWalletAccountsResult = self
            .submit_activity(
                CREATE_WALLET_ACCOUNTS_PATH,
                CREATE_WALLET_ACCOUNTS_TYPE,
                intent,
                CREATE_WALLET_ACCOUNTS_RESULT,
            )
            .await?;
        result.addresses.into_iter().next().ok_or_else(|| {
            TurnkeyError::UnexpectedResponse("create_wallet_accounts returned no address".into())
        })
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
            .process_request::<_, GetWalletAccountResponse>(GET_WALLET_ACCOUNT_PATH, &request)
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
        )
        .await
    }

    /// Exports the secret key for the account at `path`, decrypting the bundle
    /// against the pinned production quorum key. Used where Turnkey's design
    /// requires a local key (static-deposit refund, SDK-layer encryption/HMAC).
    pub(crate) async fn export_secret_key(&self, path: String) -> Result<SecretKey, TurnkeyError> {
        let address = self.create_account(path, ADDRESS_FORMAT_COMPRESSED).await?;
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
    ecdsa::Signature::from_compact(&compact).map_err(|e| TurnkeyError::Deserialize(e.to_string()))
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
