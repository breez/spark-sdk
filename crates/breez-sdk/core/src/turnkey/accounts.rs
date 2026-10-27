//! Shared Turnkey wallet-account operations and byte conversions used by both
//! the Spark signer and the SDK-layer (Breez) signer.

use std::str::FromStr;

use bitcoin::bip32::{ChainCode, ChildNumber, Fingerprint, Xpriv};
use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey, ecdsa, schnorr};
use bitcoin::{Address, KnownHrp, NetworkKind};
use spark_wallet::SparkAddress;

use crate::Network;

use turnkey_enclave_encrypt::{ExportClient, QuorumPublicKey};

use super::error::TurnkeyError;
use super::transport::{OnConflict, TurnkeyClient};
use super::types::{
    ADDRESS_FORMAT_BITCOIN_MAINNET_P2TR, ADDRESS_FORMAT_BITCOIN_REGTEST_P2TR,
    ADDRESS_FORMAT_BITCOIN_SIGNET_P2TR, ADDRESS_FORMAT_COMPRESSED, ADDRESS_FORMAT_SPARK_MAINNET,
    ADDRESS_FORMAT_SPARK_REGTEST, CREATE_WALLET_ACCOUNTS_PATH, CREATE_WALLET_ACCOUNTS_RESULT,
    CREATE_WALLET_ACCOUNTS_TYPE, CURVE_SECP256K1, CreateWalletAccountsIntent,
    CreateWalletAccountsResult, ENCODING_HEXADECIMAL, EXPORT_WALLET_ACCOUNT_PATH,
    EXPORT_WALLET_ACCOUNT_RESULT, EXPORT_WALLET_ACCOUNT_TYPE, ExportWalletAccountIntent,
    ExportWalletAccountResult, GET_WALLET_ACCOUNT_PATH, GetWalletAccountRequest,
    GetWalletAccountResponse, PATH_FORMAT_BIP32, SIGN_RAW_PAYLOAD_PATH, SIGN_RAW_PAYLOAD_RESULT,
    SIGN_RAW_PAYLOAD_TYPE, SignRawPayloadIntent, SignRawPayloadResult, WalletAccount,
    WalletAccountParams,
};

/// The Spark address format (and thus the BIP-340 Schnorr signing scheme) for
/// the wallet's network.
pub(crate) fn spark_address_format(network: Network) -> Result<&'static str, TurnkeyError> {
    match network {
        Network::Mainnet => Ok(ADDRESS_FORMAT_SPARK_MAINNET),
        Network::Regtest => Ok(ADDRESS_FORMAT_SPARK_REGTEST),
        // Turnkey exposes Spark formats only for mainnet and regtest.
        Network::Signet => Err(TurnkeyError::InvalidConfig(
            "Turnkey does not support Spark Signet accounts".to_string(),
        )),
    }
}

/// The Bitcoin P2TR address format (and thus the BIP341 tweaked-Schnorr signing
/// scheme) for the wallet's network. Used to sign leaf refund outputs in a
/// unilateral exit.
pub(crate) fn bitcoin_p2tr_format(network: Network) -> &'static str {
    match network {
        Network::Mainnet => ADDRESS_FORMAT_BITCOIN_MAINNET_P2TR,
        Network::Regtest => ADDRESS_FORMAT_BITCOIN_REGTEST_P2TR,
        Network::Signet => ADDRESS_FORMAT_BITCOIN_SIGNET_P2TR,
    }
}

impl TurnkeyClient {
    /// Addresses of the accounts at `path` with `address_formats`, in order.
    /// Existing accounts are looked up and only the missing ones are created, in
    /// one activity, so an existing account costs a query rather than a failed
    /// `CREATE_WALLET_ACCOUNTS` activity.
    pub(crate) async fn ensure_accounts(
        &self,
        path: &str,
        address_formats: &[&'static str],
    ) -> Result<Vec<String>, TurnkeyError> {
        let at_path = self.account_by_path(path).await?;
        let mut addresses = Vec::with_capacity(address_formats.len());
        let mut missing = Vec::new();
        for &address_format in address_formats {
            let address = self
                .existing_address(at_path.as_ref(), address_format)
                .await?;
            if address.is_none() {
                missing.push(address_format);
            }
            addresses.push(address);
        }
        let mut created = if missing.is_empty() {
            Vec::new()
        } else {
            self.create_accounts(path, &missing).await?
        }
        .into_iter();
        addresses
            .into_iter()
            .map(|address| {
                address.or_else(|| created.next()).ok_or_else(|| {
                    TurnkeyError::UnexpectedResponse(
                        "create_wallet_accounts returned too few addresses".into(),
                    )
                })
            })
            .collect()
    }

    /// Address of the account at `path` with `address_format`, created if the
    /// wallet has none (see [`Self::ensure_accounts`]).
    pub(crate) async fn ensure_account(
        &self,
        path: &str,
        address_format: &'static str,
    ) -> Result<String, TurnkeyError> {
        self.ensure_accounts(path, &[address_format])
            .await?
            .pop()
            .ok_or_else(|| TurnkeyError::UnexpectedResponse("no account address".into()))
    }

    /// Creates the accounts at `path` with `address_formats` in one activity,
    /// returning their addresses in order. A 409 means one of them already
    /// exists, so their addresses are read back.
    async fn create_accounts(
        &self,
        path: &str,
        address_formats: &[&'static str],
    ) -> Result<Vec<String>, TurnkeyError> {
        let intent = CreateWalletAccountsIntent {
            wallet_id: self.wallet_id.clone(),
            accounts: address_formats
                .iter()
                .map(|&address_format| WalletAccountParams {
                    curve: CURVE_SECP256K1,
                    path_format: PATH_FORMAT_BIP32,
                    path: path.to_string(),
                    address_format,
                })
                .collect(),
        };
        let result: Result<CreateWalletAccountsResult, TurnkeyError> = self
            .submit_activity(
                CREATE_WALLET_ACCOUNTS_PATH,
                CREATE_WALLET_ACCOUNTS_TYPE,
                intent,
                CREATE_WALLET_ACCOUNTS_RESULT,
                OnConflict::Surface,
            )
            .await;
        match result {
            Ok(result) if result.addresses.len() == address_formats.len() => Ok(result.addresses),
            Ok(result) => Err(TurnkeyError::UnexpectedResponse(format!(
                "create_wallet_accounts returned {} addresses for {} accounts",
                result.addresses.len(),
                address_formats.len()
            ))),
            Err(conflict @ TurnkeyError::Http { status: 409, .. }) => {
                let at_path = self.account_by_path(path).await?;
                let mut addresses = Vec::with_capacity(address_formats.len());
                for &address_format in address_formats {
                    match self
                        .existing_address(at_path.as_ref(), address_format)
                        .await?
                    {
                        Some(address) => addresses.push(address),
                        None => return Err(conflict),
                    }
                }
                Ok(addresses)
            }
            Err(e) => Err(e),
        }
    }

    /// Address of the existing account with `address_format` at the path that
    /// `at_path` was looked up by. A by-path lookup returns one account even when
    /// several formats share the path, so another format is looked up by the
    /// address derived from their shared key. `None` when there is none, or when
    /// the format's address can't be derived.
    async fn existing_address(
        &self,
        at_path: Option<&WalletAccount>,
        address_format: &str,
    ) -> Result<Option<String>, TurnkeyError> {
        let Some(account) = at_path else {
            return Ok(None);
        };
        if account.address_format == address_format {
            return Ok(Some(account.address.clone()));
        }
        let Some(address) = account
            .public_key
            .as_deref()
            .map(|key| derived_address(key, address_format))
            .transpose()?
            .flatten()
        else {
            return Ok(None);
        };
        Ok(self
            .account_by_address(&address)
            .await?
            .map(|account| account.address))
    }

    async fn account_by_path(&self, path: &str) -> Result<Option<WalletAccount>, TurnkeyError> {
        self.wallet_account(Some(path.to_string()), None).await
    }

    async fn account_by_address(
        &self,
        address: &str,
    ) -> Result<Option<WalletAccount>, TurnkeyError> {
        self.wallet_account(None, Some(address.to_string())).await
    }

    /// The wallet account at `path` or with `address`, or `None` when Turnkey
    /// has none (a 404).
    async fn wallet_account(
        &self,
        path: Option<String>,
        address: Option<String>,
    ) -> Result<Option<WalletAccount>, TurnkeyError> {
        let request = GetWalletAccountRequest {
            organization_id: self.organization_id.clone(),
            wallet_id: self.wallet_id.clone(),
            path,
            address,
        };
        match self
            .process_request::<_, GetWalletAccountResponse>(
                GET_WALLET_ACCOUNT_PATH,
                &request,
                OnConflict::Surface,
            )
            .await
        {
            Ok(resp) => Ok(Some(resp.account)),
            Err(TurnkeyError::Http { status: 404, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Compressed public key (hex) for `path`: prefers an existing account's
    /// `publicKey`, else creates an `ADDRESS_FORMAT_COMPRESSED` account (whose
    /// address is the compressed key).
    pub(crate) async fn compressed_pubkey_at(&self, path: String) -> Result<String, TurnkeyError> {
        if let Some(public_key) = self
            .account_by_path(&path)
            .await?
            .and_then(|account| account.public_key)
        {
            return Ok(public_key);
        }
        self.ensure_account(&path, ADDRESS_FORMAT_COMPRESSED).await
    }

    /// Internal (untweaked) public key (hex) for a P2TR account at `path`,
    /// provisioning it P2TR-format if absent. Creating it P2TR (not compressed)
    /// lets the same path later sign its refund output as a tweaked BIP341
    /// key-path spend for a unilateral exit, with one account per leaf path.
    /// Creating returns the taproot address, so the key is read back.
    pub(crate) async fn p2tr_pubkey_at(
        &self,
        path: String,
        network: Network,
    ) -> Result<String, TurnkeyError> {
        if let Some(public_key) = self
            .account_by_path(&path)
            .await?
            .and_then(|account| account.public_key)
        {
            return Ok(public_key);
        }
        self.ensure_account(&path, bitcoin_p2tr_format(network))
            .await?;
        self.account_by_path(&path)
            .await?
            .and_then(|account| account.public_key)
            .ok_or_else(|| {
                TurnkeyError::UnexpectedResponse(
                    "P2TR wallet account did not expose a public key".into(),
                )
            })
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
        let address = self.ensure_account(&path, address_format).await?;
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

/// The address Turnkey gives an `address_format` account whose compressed key
/// is `public_key` (hex), for the formats the SDK creates. `None` for other
/// formats.
fn derived_address(public_key: &str, address_format: &str) -> Result<Option<String>, TurnkeyError> {
    let key = PublicKey::from_str(public_key)
        .map_err(|e| TurnkeyError::Deserialize(format!("account public key: {e}")))?;
    let spark = |network: Network| {
        SparkAddress::new(key, network.into(), None)
            .to_address_string()
            .map_err(|e| TurnkeyError::Deserialize(format!("spark address: {e}")))
    };
    // A key-path-only taproot output (no script tree).
    let p2tr = |hrp: KnownHrp| {
        Address::p2tr(
            &Secp256k1::verification_only(),
            key.x_only_public_key().0,
            None,
            hrp,
        )
        .to_string()
    };
    let address = match address_format {
        ADDRESS_FORMAT_COMPRESSED => hex::encode(key.serialize()),
        ADDRESS_FORMAT_SPARK_MAINNET => spark(Network::Mainnet)?,
        ADDRESS_FORMAT_SPARK_REGTEST => spark(Network::Regtest)?,
        ADDRESS_FORMAT_BITCOIN_MAINNET_P2TR => p2tr(KnownHrp::Mainnet),
        ADDRESS_FORMAT_BITCOIN_REGTEST_P2TR => p2tr(KnownHrp::Regtest),
        ADDRESS_FORMAT_BITCOIN_SIGNET_P2TR => p2tr(KnownHrp::Testnets),
        _ => return Ok(None),
    };
    Ok(Some(address))
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
            Network::Signet | Network::Regtest => NetworkKind::Test,
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
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use bitcoin::secp256k1::ecdsa::{RecoverableSignature, RecoveryId};
    use bitcoin::secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
    use platform_utils::{HttpClient, HttpError, HttpResponse};
    use serde_json::{Value, json};

    use super::*;
    use crate::signer::ExternalSigningSigner;
    use crate::signer::external_types::MessageBytes;
    use crate::turnkey::breez_signer::TurnkeySigningSigner;
    use crate::turnkey::config::TurnkeyConfig;

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

    const PATH: &str = "m/8797555'/0'/0'";
    const IDENTITY_FORMATS: [&str; 2] = [ADDRESS_FORMAT_COMPRESSED, ADDRESS_FORMAT_SPARK_REGTEST];

    // A wallet key and the Spark regtest address Turnkey gave its account.
    const TURNKEY_KEY: &str = "02c89215604d6ad04820575399638e18ff2eff51984a3f565445821046404db492";
    const TURNKEY_SPARK_ADDRESS: &str =
        "sparkrt1pgss9jyjz4sy66ksfqs9w5uevw8p3lewlagesj3l2e2ytqssgeqymdyj3lzm9u";
    // A wallet key and the Spark mainnet address Turnkey gave its account.
    const TURNKEY_MAINNET_KEY: &str =
        "029fa7c2c5a5ea2a84155c54d05aba641465ff7113fd970799e2aea63c1b8de632";
    const TURNKEY_MAINNET_SPARK_ADDRESS: &str =
        "spark1pgss98a8ctz6t632ss24c4xst2axg9r9lac38lvhq7v79t4x8sdcme3jcqeruk";
    // A wallet key and the P2TR addresses Turnkey gave its accounts.
    const TURNKEY_P2TR_KEY: &str =
        "038a541035a0158cf2dfcdfbec6d97a5d08ca092c86baa8f8fdface4b2900abe51";
    const TURNKEY_P2TR_ADDRESSES: [(&str, &str); 3] = [
        (
            ADDRESS_FORMAT_BITCOIN_MAINNET_P2TR,
            "bc1phs76mm756utjtvmcs6ug2w2gna55fy75lxmyezz28sgvxjpsn7ds4k0mua",
        ),
        (
            ADDRESS_FORMAT_BITCOIN_REGTEST_P2TR,
            "bcrt1phs76mm756utjtvmcs6ug2w2gna55fy75lxmyezz28sgvxjpsn7ds08njng",
        ),
        (
            ADDRESS_FORMAT_BITCOIN_SIGNET_P2TR,
            "tb1phs76mm756utjtvmcs6ug2w2gna55fy75lxmyezz28sgvxjpsn7dsz7e5xj",
        ),
    ];

    /// A Turnkey wallet in memory: answers the get-account, create-accounts and
    /// sign endpoints as Turnkey does (a by-path lookup returns the oldest
    /// account at the path, and signing needs an account with the `signWith`
    /// address) and records each request's endpoint.
    struct FakeWallet {
        accounts: Mutex<Vec<(String, String, String)>>,
        endpoints: Mutex<Vec<String>>,
        signed_with: Mutex<Vec<String>>,
        lookup_status: Option<u16>,
    }

    impl FakeWallet {
        /// A wallet with accounts at `PATH` in `formats`, oldest first.
        fn with(formats: &[&str]) -> Arc<Self> {
            Self::build(formats, None)
        }

        /// A wallet whose lookups all fail with `status`.
        fn failing_lookups(status: u16) -> Arc<Self> {
            Self::build(&[], Some(status))
        }

        fn build(formats: &[&str], lookup_status: Option<u16>) -> Arc<Self> {
            let accounts = formats
                .iter()
                .map(|format| (PATH.to_string(), (*format).to_string(), address(format)))
                .collect();
            Arc::new(Self {
                accounts: Mutex::new(accounts),
                endpoints: Mutex::new(Vec::new()),
                signed_with: Mutex::new(Vec::new()),
                lookup_status,
            })
        }

        fn formats(&self) -> Vec<String> {
            let accounts = self.accounts.lock().unwrap();
            accounts
                .iter()
                .map(|(_, format, _)| format.clone())
                .collect()
        }

        fn creates(&self) -> usize {
            let endpoints = self.endpoints.lock().unwrap();
            endpoints
                .iter()
                .filter(|endpoint| *endpoint == "create_wallet_accounts")
                .count()
        }

        fn respond(&self, endpoint: &str, body: &Value) -> HttpResponse {
            self.endpoints.lock().unwrap().push(endpoint.to_string());
            let mut accounts = self.accounts.lock().unwrap();
            match endpoint {
                "get_wallet_account" if self.lookup_status.is_some() => response(
                    self.lookup_status.unwrap_or_default(),
                    &json!({"message": "lookup failed"}),
                ),
                "get_wallet_account" => {
                    let found = accounts.iter().find(|(path, _, address)| {
                        body["path"].as_str() == Some(path.as_str())
                            || body["address"].as_str() == Some(address.as_str())
                    });
                    match found {
                        Some((path, format, address)) => response(
                            200,
                            &json!({"account": {
                                "path": path,
                                "addressFormat": format,
                                "address": address,
                                "publicKey": TURNKEY_KEY,
                            }}),
                        ),
                        None => response(
                            404,
                            &json!({"code": 5, "message": "wallet account not found"}),
                        ),
                    }
                }
                "create_wallet_accounts" => {
                    let requested: Vec<(String, String)> = body["parameters"]["accounts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|account| {
                            (
                                account["path"].as_str().unwrap().to_string(),
                                account["addressFormat"].as_str().unwrap().to_string(),
                            )
                        })
                        .collect();
                    let exists = requested.iter().any(|(path, format)| {
                        accounts.iter().any(|(p, f, _)| p == path && f == format)
                    });
                    if exists {
                        return response(
                            409,
                            &json!({"code": 6, "message": "path already exists in wallet account"}),
                        );
                    }
                    let addresses: Vec<String> = requested
                        .into_iter()
                        .map(|(path, format)| {
                            let address = address(&format);
                            accounts.push((path, format, address.clone()));
                            address
                        })
                        .collect();
                    response(
                        200,
                        &json!({"activity": {
                            "id": "activity-id",
                            "status": "ACTIVITY_STATUS_COMPLETED",
                            "result": {"createWalletAccountsResult": {"addresses": addresses}},
                        }}),
                    )
                }
                "sign_raw_payload" => {
                    let sign_with = body["parameters"]["signWith"].as_str().unwrap();
                    if !accounts.iter().any(|(_, _, address)| address == sign_with) {
                        return response(
                            404,
                            &json!({"code": 5, "message": "wallet account not found"}),
                        );
                    }
                    self.signed_with.lock().unwrap().push(sign_with.to_string());
                    let scalar = "01".repeat(32);
                    response(
                        200,
                        &json!({"activity": {
                            "id": "activity-id",
                            "status": "ACTIVITY_STATUS_COMPLETED",
                            "result": {"signRawPayloadResult": {"r": scalar, "s": scalar, "v": "00"}},
                        }}),
                    )
                }
                other => panic!("unexpected Turnkey endpoint {other}"),
            }
        }
    }

    #[macros::async_trait]
    impl HttpClient for FakeWallet {
        async fn get(
            &self,
            url: String,
            _headers: Option<HashMap<String, String>>,
        ) -> Result<HttpResponse, HttpError> {
            panic!("unexpected GET {url}")
        }
        async fn post(
            &self,
            url: String,
            _headers: Option<HashMap<String, String>>,
            body: Option<String>,
        ) -> Result<HttpResponse, HttpError> {
            let endpoint = url.rsplit('/').next().unwrap_or_default().to_string();
            let body: Value = serde_json::from_str(&body.unwrap_or_default()).unwrap();
            Ok(self.respond(&endpoint, &body))
        }
        async fn delete(
            &self,
            url: String,
            _headers: Option<HashMap<String, String>>,
            _body: Option<String>,
        ) -> Result<HttpResponse, HttpError> {
            panic!("unexpected DELETE {url}")
        }
    }

    /// The address Turnkey gives an account of `format` for `TURNKEY_KEY`. P2TR
    /// uses the SDK's derivation, which `derived_addresses_match_turnkey` checks.
    fn address(format: &str) -> String {
        match format {
            ADDRESS_FORMAT_COMPRESSED => TURNKEY_KEY.to_string(),
            ADDRESS_FORMAT_SPARK_REGTEST => TURNKEY_SPARK_ADDRESS.to_string(),
            other => derived_address(TURNKEY_KEY, other)
                .unwrap()
                .unwrap_or_else(|| format!("{other}-address")),
        }
    }

    fn response(status: u16, body: &Value) -> HttpResponse {
        HttpResponse {
            status,
            body: body.to_string(),
            headers: HashMap::new(),
        }
    }

    fn client(wallet: Arc<FakeWallet>) -> TurnkeyClient {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x11; 32]).unwrap();
        let config = TurnkeyConfig {
            base_url: Some("https://turnkey.invalid".to_string()),
            organization_id: "test-org".to_string(),
            api_public_key: hex::encode(sk.public_key(&secp).serialize()),
            api_private_key: hex::encode(sk.secret_bytes()),
            wallet_id: "test-wallet".to_string(),
            network: Network::Regtest,
            account_number: Some(0),
            identity_public_key: None,
            retry: None,
            max_rps: Some(1000),
            proxy: None,
        };
        TurnkeyClient::new(&config, wallet).unwrap()
    }

    #[test]
    fn derived_addresses_match_turnkey() {
        let derive = |format| derived_address(TURNKEY_KEY, format).unwrap();
        assert_eq!(
            derive(ADDRESS_FORMAT_COMPRESSED).as_deref(),
            Some(TURNKEY_KEY)
        );
        assert_eq!(
            derive(ADDRESS_FORMAT_SPARK_REGTEST).as_deref(),
            Some(TURNKEY_SPARK_ADDRESS)
        );
        assert_eq!(derive("ADDRESS_FORMAT_ETHEREUM"), None);
        assert_eq!(
            derived_address(TURNKEY_MAINNET_KEY, ADDRESS_FORMAT_SPARK_MAINNET)
                .unwrap()
                .as_deref(),
            Some(TURNKEY_MAINNET_SPARK_ADDRESS)
        );
        for (format, address) in TURNKEY_P2TR_ADDRESSES {
            assert_eq!(
                derived_address(TURNKEY_P2TR_KEY, format)
                    .unwrap()
                    .as_deref(),
                Some(address)
            );
        }
    }

    // Both identity accounts exist, in either creation order.
    #[tokio::test]
    async fn existing_accounts_are_looked_up_not_created() {
        for formats in [IDENTITY_FORMATS, [IDENTITY_FORMATS[1], IDENTITY_FORMATS[0]]] {
            let wallet = FakeWallet::with(&formats);
            let addresses = client(wallet.clone())
                .ensure_accounts(PATH, &IDENTITY_FORMATS)
                .await
                .unwrap();
            assert_eq!(addresses, [TURNKEY_KEY, TURNKEY_SPARK_ADDRESS]);
            assert_eq!(wallet.creates(), 0);
        }
    }

    #[tokio::test]
    async fn missing_accounts_are_created_in_one_activity() {
        let wallet = FakeWallet::with(&[]);
        let addresses = client(wallet.clone())
            .ensure_accounts(PATH, &IDENTITY_FORMATS)
            .await
            .unwrap();
        assert_eq!(addresses, [TURNKEY_KEY, TURNKEY_SPARK_ADDRESS]);
        assert_eq!(wallet.creates(), 1);
        assert_eq!(wallet.formats(), IDENTITY_FORMATS);
    }

    // A wallet with only the Spark-format account gets just the compressed one.
    #[tokio::test]
    async fn only_the_missing_format_is_created() {
        let wallet = FakeWallet::with(&[ADDRESS_FORMAT_SPARK_REGTEST]);
        client(wallet.clone())
            .ensure_accounts(PATH, &IDENTITY_FORMATS)
            .await
            .unwrap();
        assert_eq!(wallet.creates(), 1);
        assert_eq!(
            wallet.formats(),
            [ADDRESS_FORMAT_SPARK_REGTEST, ADDRESS_FORMAT_COMPRESSED]
        );
    }

    // A create that loses a race reads back the requested format, not the
    // oldest account at the path.
    #[tokio::test]
    async fn conflict_returns_the_requested_format() {
        let wallet = FakeWallet::with(&IDENTITY_FORMATS);
        let addresses = client(wallet)
            .create_accounts(PATH, &[ADDRESS_FORMAT_SPARK_REGTEST])
            .await
            .unwrap();
        assert_eq!(addresses, [TURNKEY_SPARK_ADDRESS]);
    }

    // A leaf path from an SDK version that created leaf accounts compressed,
    // with the P2TR account added later.
    #[tokio::test]
    async fn p2tr_behind_a_compressed_account_is_found() {
        let wallet = FakeWallet::with(&[
            ADDRESS_FORMAT_COMPRESSED,
            ADDRESS_FORMAT_BITCOIN_REGTEST_P2TR,
        ]);
        let found = client(wallet.clone())
            .ensure_account(PATH, ADDRESS_FORMAT_BITCOIN_REGTEST_P2TR)
            .await
            .unwrap();
        assert_eq!(found, address(ADDRESS_FORMAT_BITCOIN_REGTEST_P2TR));
        assert_eq!(wallet.creates(), 0);
    }

    #[tokio::test]
    async fn lookup_errors_other_than_not_found_are_returned() {
        let wallet = FakeWallet::failing_lookups(403);
        let result = client(wallet.clone())
            .ensure_accounts(PATH, &IDENTITY_FORMATS)
            .await;
        assert!(matches!(
            result,
            Err(TurnkeyError::Http { status: 403, .. })
        ));
        assert_eq!(wallet.creates(), 0);
    }

    // ECDSA at a path holding only the Spark-format account signs with a
    // compressed account, created for it.
    #[tokio::test]
    async fn ecdsa_signs_with_a_compressed_account() {
        let wallet = FakeWallet::with(&[ADDRESS_FORMAT_SPARK_REGTEST]);
        let signer =
            TurnkeySigningSigner::new(Arc::new(client(wallet.clone())), Network::Regtest, 0);
        signer
            .sign_ecdsa(MessageBytes::new(vec![0x42; 32]), "m".to_string())
            .await
            .unwrap();
        assert_eq!(*wallet.signed_with.lock().unwrap(), [TURNKEY_KEY]);
        assert_eq!(
            wallet.formats(),
            [ADDRESS_FORMAT_SPARK_REGTEST, ADDRESS_FORMAT_COMPRESSED]
        );
    }
}
