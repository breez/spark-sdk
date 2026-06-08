//! `SparkSigner` implementation backed by Turnkey.
//!
//! Spark keys are hardened children of `m/8797555'/{account}'`: identity at
//! `/0'`, signing leaf at `/1'/{u32_be(sha256(leaf_id)[..4]) % 2^31}'`, static
//! deposit at `/3'/{index}'`. The signing flows pass Turnkey opaque derivation
//! tags rather than paths; the pure-pubkey methods materialize the account at
//! the path (deterministic from the wallet seed, so recoverable without local
//! state) and read its compressed key.
//!
//! The pubkey maps are a performance memoization only. Each derivation's key is
//! deterministic and immutable, so caching avoids repeat Turnkey round-trips
//! (and re-issuing a `CREATE_WALLET_ACCOUNTS` activity for a path already
//! materialized), and never needs invalidation. They are not persistence:
//! in-memory, lost on restart, rebuilt from Turnkey on demand. Correctness does
//! not depend on them.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::{PublicKey, ecdsa};
use spark_wallet::{
    FrostJob, FrostShareResult, PrepareClaimRequest, PrepareLightningReceiveRequest,
    PrepareStaticDepositClaimRequest, PrepareStaticDepositRequest, PrepareTokenTransactionRequest,
    PrepareTransferRequest, PreparedClaim, PreparedLightningReceive, PreparedStaticDeposit,
    PreparedStaticDepositClaim, PreparedTokenTransaction, PreparedTransfer,
    SignSparkInvoiceRequest, SignStaticDepositRefundRequest, SignedSparkInvoice, SignerError,
    SparkSigner, StartStaticDepositRefundRequest, StartedStaticDepositRefund, TreeNodeId,
};

use super::error::TurnkeyError;
use super::transport::TurnkeyClient;
use super::types::{
    ADDRESS_FORMAT_COMPRESSED, CREATE_WALLET_ACCOUNTS_PATH, CREATE_WALLET_ACCOUNTS_RESULT,
    CREATE_WALLET_ACCOUNTS_TYPE, CURVE_SECP256K1, CreateWalletAccountsIntent,
    CreateWalletAccountsResult, ENCODING_HEXADECIMAL, GET_WALLET_ACCOUNT_PATH,
    GetWalletAccountRequest, GetWalletAccountResponse, HASH_FUNCTION_SHA256, PATH_FORMAT_BIP32,
    SIGN_RAW_PAYLOAD_PATH, SIGN_RAW_PAYLOAD_RESULT, SIGN_RAW_PAYLOAD_TYPE, SignRawPayloadIntent,
    SignRawPayloadResult, WalletAccountParams,
};

fn to_spark_err<E: std::fmt::Display>(e: E) -> SignerError {
    SignerError::Generic(e.to_string())
}

/// Decodes a hex scalar into 32 bytes, left-padding if Turnkey omitted leading
/// zeros.
fn decode_scalar_32(hex_str: &str) -> Result<[u8; 32], TurnkeyError> {
    let bytes =
        hex::decode(hex_str).map_err(|e| TurnkeyError::Deserialize(format!("scalar hex: {e}")))?;
    if bytes.len() > 32 {
        return Err(TurnkeyError::Deserialize("scalar exceeds 32 bytes".into()));
    }
    let mut out = [0u8; 32];
    out[32 - bytes.len()..].copy_from_slice(&bytes);
    Ok(out)
}

fn ecdsa_from_rs(r_hex: &str, s_hex: &str) -> Result<ecdsa::Signature, TurnkeyError> {
    let mut compact = [0u8; 64];
    compact[..32].copy_from_slice(&decode_scalar_32(r_hex)?);
    compact[32..].copy_from_slice(&decode_scalar_32(s_hex)?);
    ecdsa::Signature::from_compact(&compact).map_err(|e| TurnkeyError::Deserialize(e.to_string()))
}

pub(crate) struct TurnkeySparkSigner {
    client: Arc<TurnkeyClient>,
    account: u32,
    // Pubkey memoization only (see module docs): immutable, in-memory, non-authoritative.
    identity_pubkey: Mutex<Option<PublicKey>>,
    leaf_pubkeys: Mutex<HashMap<TreeNodeId, PublicKey>>,
    static_deposit_pubkeys: Mutex<HashMap<u32, PublicKey>>,
}

impl TurnkeySparkSigner {
    pub(crate) fn new(client: Arc<TurnkeyClient>) -> Self {
        Self {
            client,
            account: 0,
            identity_pubkey: Mutex::new(None),
            leaf_pubkeys: Mutex::new(HashMap::new()),
            static_deposit_pubkeys: Mutex::new(HashMap::new()),
        }
    }

    fn base_path(&self) -> String {
        format!("m/8797555'/{}'", self.account)
    }

    fn leaf_index(leaf_id: &TreeNodeId) -> u32 {
        let hash = sha256::Hash::hash(leaf_id.to_string().as_bytes());
        let bytes: [u8; 4] = hash.as_byte_array()[..4]
            .try_into()
            .expect("sha256 digest is 32 bytes");
        u32::from_be_bytes(bytes) % (1 << 31)
    }

    async fn pubkey_at_path(&self, path: String) -> Result<PublicKey, SignerError> {
        let hex = self
            .fetch_or_create_pubkey_hex(path)
            .await
            .map_err(to_spark_err)?;
        PublicKey::from_str(&hex).map_err(to_spark_err)
    }

    /// Reads the compressed public key for a derivation path, preferring an
    /// existing account's `publicKey` and otherwise materializing the account
    /// with `ADDRESS_FORMAT_COMPRESSED` (whose address is the compressed key).
    async fn fetch_or_create_pubkey_hex(&self, path: String) -> Result<String, TurnkeyError> {
        let request = GetWalletAccountRequest {
            organization_id: self.client.organization_id.clone(),
            wallet_id: self.client.wallet_id.clone(),
            path: path.clone(),
        };
        if let Ok(resp) = self
            .client
            .process_request::<_, GetWalletAccountResponse>(GET_WALLET_ACCOUNT_PATH, &request)
            .await
            && let Some(public_key) = resp.account.public_key
        {
            return Ok(public_key);
        }

        let intent = CreateWalletAccountsIntent {
            wallet_id: self.client.wallet_id.clone(),
            accounts: vec![WalletAccountParams {
                curve: CURVE_SECP256K1,
                path_format: PATH_FORMAT_BIP32,
                path,
                address_format: ADDRESS_FORMAT_COMPRESSED,
            }],
        };
        let result: CreateWalletAccountsResult = self
            .client
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

    /// ECDSA-signs `message` with the identity key via `SIGN_RAW_PAYLOAD`.
    /// `signWith` is the compressed identity address, which selects secp256k1
    /// ECDSA (the Spark-format address would instead select BIP-340 Schnorr).
    /// Turnkey applies SHA-256 to the payload, matching the local signer.
    async fn sign_identity_ecdsa(&self, message: &[u8]) -> Result<ecdsa::Signature, SignerError> {
        let identity = self.get_identity_public_key().await?;
        let intent = SignRawPayloadIntent {
            sign_with: hex::encode(identity.serialize()),
            payload: hex::encode(message),
            encoding: ENCODING_HEXADECIMAL,
            hash_function: HASH_FUNCTION_SHA256,
        };
        let result: SignRawPayloadResult = self
            .client
            .submit_activity(
                SIGN_RAW_PAYLOAD_PATH,
                SIGN_RAW_PAYLOAD_TYPE,
                intent,
                SIGN_RAW_PAYLOAD_RESULT,
            )
            .await
            .map_err(to_spark_err)?;
        ecdsa_from_rs(&result.r, &result.s).map_err(to_spark_err)
    }
}

#[macros::async_trait]
impl spark_wallet::SparkSigner for TurnkeySparkSigner {
    async fn get_identity_public_key(&self) -> Result<PublicKey, SignerError> {
        if let Some(pk) = *self.identity_pubkey.lock().unwrap() {
            return Ok(pk);
        }
        let pk = self
            .pubkey_at_path(format!("{}/0'", self.base_path()))
            .await?;
        *self.identity_pubkey.lock().unwrap() = Some(pk);
        Ok(pk)
    }

    async fn get_public_key_for_leaf(
        &self,
        leaf_id: &TreeNodeId,
    ) -> Result<PublicKey, SignerError> {
        if let Some(pk) = self.leaf_pubkeys.lock().unwrap().get(leaf_id).copied() {
            return Ok(pk);
        }
        let path = format!("{}/1'/{}'", self.base_path(), Self::leaf_index(leaf_id));
        let pk = self.pubkey_at_path(path).await?;
        self.leaf_pubkeys
            .lock()
            .unwrap()
            .insert(leaf_id.clone(), pk);
        Ok(pk)
    }

    async fn get_static_deposit_public_key(&self, index: u32) -> Result<PublicKey, SignerError> {
        if let Some(pk) = self
            .static_deposit_pubkeys
            .lock()
            .unwrap()
            .get(&index)
            .copied()
        {
            return Ok(pk);
        }
        let path = format!("{}/3'/{index}'", self.base_path());
        let pk = self.pubkey_at_path(path).await?;
        self.static_deposit_pubkeys
            .lock()
            .unwrap()
            .insert(index, pk);
        Ok(pk)
    }

    async fn sign_authentication_challenge(
        &self,
        challenge: &[u8],
    ) -> Result<ecdsa::Signature, SignerError> {
        self.sign_identity_ecdsa(challenge).await
    }

    async fn sign_message(&self, message: &[u8]) -> Result<ecdsa::Signature, SignerError> {
        self.sign_identity_ecdsa(message).await
    }

    async fn sign_frost(&self, _jobs: Vec<FrostJob>) -> Result<Vec<FrostShareResult>, SignerError> {
        Err(SignerError::Generic(
            "turnkey signer: sign_frost not yet implemented".to_string(),
        ))
    }

    async fn prepare_transfer(
        &self,
        _request: PrepareTransferRequest,
    ) -> Result<PreparedTransfer, SignerError> {
        Err(SignerError::Generic(
            "turnkey signer: prepare_transfer not yet implemented".to_string(),
        ))
    }

    async fn prepare_claim(
        &self,
        _request: PrepareClaimRequest,
    ) -> Result<PreparedClaim, SignerError> {
        Err(SignerError::Generic(
            "turnkey signer: prepare_claim not yet implemented".to_string(),
        ))
    }

    async fn prepare_lightning_receive(
        &self,
        _request: PrepareLightningReceiveRequest,
    ) -> Result<PreparedLightningReceive, SignerError> {
        Err(SignerError::Generic(
            "turnkey signer: prepare_lightning_receive not yet implemented".to_string(),
        ))
    }

    async fn prepare_static_deposit(
        &self,
        _request: PrepareStaticDepositRequest,
    ) -> Result<PreparedStaticDeposit, SignerError> {
        Err(SignerError::Generic(
            "turnkey signer: prepare_static_deposit not yet implemented".to_string(),
        ))
    }

    async fn start_static_deposit_refund(
        &self,
        _request: StartStaticDepositRefundRequest,
    ) -> Result<StartedStaticDepositRefund, SignerError> {
        Err(SignerError::Generic(
            "turnkey signer: start_static_deposit_refund not yet implemented".to_string(),
        ))
    }

    async fn sign_static_deposit_refund(
        &self,
        _request: SignStaticDepositRefundRequest,
    ) -> Result<frost_secp256k1_tr::Signature, SignerError> {
        Err(SignerError::Generic(
            "turnkey signer: sign_static_deposit_refund not yet implemented".to_string(),
        ))
    }

    async fn prepare_static_deposit_claim(
        &self,
        _request: PrepareStaticDepositClaimRequest,
    ) -> Result<PreparedStaticDepositClaim, SignerError> {
        Err(SignerError::Generic(
            "turnkey signer: prepare_static_deposit_claim not yet implemented".to_string(),
        ))
    }

    async fn sign_spark_invoice(
        &self,
        _request: SignSparkInvoiceRequest,
    ) -> Result<SignedSparkInvoice, SignerError> {
        Err(SignerError::Generic(
            "turnkey signer: sign_spark_invoice not yet implemented".to_string(),
        ))
    }

    async fn prepare_token_transaction(
        &self,
        _request: PrepareTokenTransactionRequest,
    ) -> Result<PreparedTokenTransaction, SignerError> {
        Err(SignerError::Generic(
            "turnkey signer: prepare_token_transaction not yet implemented".to_string(),
        ))
    }
}
