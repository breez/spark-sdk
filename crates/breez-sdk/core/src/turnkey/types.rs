//! Typed request/result structs for the Turnkey activities and queries the
//! signer uses. JSON keys are camelCase; enum values are Turnkey's
//! `SCREAMING_SNAKE` strings.

use serde::{Deserialize, Serialize};

pub(crate) const CURVE_SECP256K1: &str = "CURVE_SECP256K1";
pub(crate) const PATH_FORMAT_BIP32: &str = "PATH_FORMAT_BIP32";
pub(crate) const ADDRESS_FORMAT_COMPRESSED: &str = "ADDRESS_FORMAT_COMPRESSED";

pub(crate) const CREATE_WALLET_ACCOUNTS_PATH: &str = "/public/v1/submit/create_wallet_accounts";
pub(crate) const CREATE_WALLET_ACCOUNTS_TYPE: &str = "ACTIVITY_TYPE_CREATE_WALLET_ACCOUNTS";
pub(crate) const CREATE_WALLET_ACCOUNTS_RESULT: &str = "createWalletAccountsResult";
pub(crate) const GET_WALLET_ACCOUNT_PATH: &str = "/public/v1/query/get_wallet_account";

pub(crate) const SIGN_RAW_PAYLOAD_PATH: &str = "/public/v1/submit/sign_raw_payload";
pub(crate) const SIGN_RAW_PAYLOAD_TYPE: &str = "ACTIVITY_TYPE_SIGN_RAW_PAYLOAD_V2";
pub(crate) const SIGN_RAW_PAYLOAD_RESULT: &str = "signRawPayloadResult";
pub(crate) const ENCODING_HEXADECIMAL: &str = "PAYLOAD_ENCODING_HEXADECIMAL";
pub(crate) const HASH_FUNCTION_SHA256: &str = "HASH_FUNCTION_SHA256";
pub(crate) const HASH_FUNCTION_NO_OP: &str = "HASH_FUNCTION_NO_OP";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WalletAccountParams {
    pub curve: &'static str,
    pub path_format: &'static str,
    pub path: String,
    pub address_format: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateWalletAccountsIntent {
    pub wallet_id: String,
    pub accounts: Vec<WalletAccountParams>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateWalletAccountsResult {
    pub addresses: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GetWalletAccountRequest {
    pub organization_id: String,
    pub wallet_id: String,
    pub path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GetWalletAccountResponse {
    pub account: WalletAccount,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WalletAccount {
    pub address: String,
    #[serde(default)]
    pub public_key: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignRawPayloadIntent {
    pub sign_with: String,
    pub payload: String,
    pub encoding: &'static str,
    pub hash_function: &'static str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignRawPayloadResult {
    pub r: String,
    pub s: String,
    /// ECDSA recovery id (hex). Absent for Schnorr, so defaulted.
    #[serde(default)]
    pub v: String,
}

pub(crate) const ADDRESS_FORMAT_SPARK_MAINNET: &str = "ADDRESS_FORMAT_SPARK_MAINNET";
pub(crate) const ADDRESS_FORMAT_SPARK_REGTEST: &str = "ADDRESS_FORMAT_SPARK_REGTEST";

pub(crate) const SPARK_SIGN_FROST_PATH: &str = "/public/v1/submit/spark_sign_frost";
pub(crate) const SPARK_SIGN_FROST_TYPE: &str = "ACTIVITY_TYPE_SPARK_SIGN_FROST";
pub(crate) const SPARK_SIGN_FROST_RESULT: &str = "sparkSignFrostResult";

#[derive(Serialize)]
struct EmptyObject {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SigningLeafDerivation {
    leaf_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StaticDepositDerivation {
    index: u32,
}

/// Spark key selector. Serialized as the flat camelCase oneof the live API
/// expects (e.g. `{"signingLeaf":{"leafId":"..."}}`), exactly one field set.
#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkKeyDerivation {
    #[serde(skip_serializing_if = "Option::is_none")]
    identity: Option<EmptyObject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signing_leaf: Option<SigningLeafDerivation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deposit: Option<EmptyObject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    static_deposit: Option<StaticDepositDerivation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    htlc_preimage: Option<EmptyObject>,
}

impl SparkKeyDerivation {
    pub(crate) fn identity() -> Self {
        Self {
            identity: Some(EmptyObject {}),
            ..Default::default()
        }
    }

    pub(crate) fn signing_leaf(leaf_id: String) -> Self {
        Self {
            signing_leaf: Some(SigningLeafDerivation { leaf_id }),
            ..Default::default()
        }
    }

    pub(crate) fn static_deposit(index: u32) -> Self {
        Self {
            static_deposit: Some(StaticDepositDerivation { index }),
            ..Default::default()
        }
    }

    pub(crate) fn htlc_preimage() -> Self {
        Self {
            htlc_preimage: Some(EmptyObject {}),
            ..Default::default()
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkSignFrostIntent {
    pub sign_with: String,
    pub signatures: Vec<SparkSignatureRequest>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkSignatureRequest {
    pub derivation: SparkKeyDerivation,
    pub message: String,
    pub verifying_key: String,
    pub operator_commitments: Vec<SparkFrostCommitment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adaptor_public_key: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkFrostCommitment {
    pub id: String,
    pub hiding: String,
    pub binding: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkSignFrostResult {
    pub signatures: Vec<SparkPartialSignature>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkPartialSignature {
    pub signature_share: String,
    pub hiding: String,
    pub binding: String,
}

pub(crate) const SPARK_PREPARE_LIGHTNING_RECEIVE_PATH: &str =
    "/public/v1/submit/spark_prepare_lightning_receive";
pub(crate) const SPARK_PREPARE_LIGHTNING_RECEIVE_TYPE: &str =
    "ACTIVITY_TYPE_SPARK_PREPARE_LIGHTNING_RECEIVE";
pub(crate) const SPARK_PREPARE_LIGHTNING_RECEIVE_RESULT: &str =
    "sparkPrepareLightningReceiveResult";

/// An operator the signer encrypts shares to. `operatorId` is the hex FROST
/// identifier (the same value used for `operatorCommitments[].id`).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkOperatorRecipient {
    pub operator_id: String,
    pub encryption_public_key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkEncryptedOperatorPackage {
    pub operator_id: String,
    pub encrypted_package: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkLightningReceivePackage {
    pub threshold: u32,
    pub operator_recipients: Vec<SparkOperatorRecipient>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkPrepareLightningReceiveIntent {
    pub sign_with: String,
    pub lightning_receive: SparkLightningReceivePackage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkPrepareLightningReceiveResult {
    #[serde(default)]
    pub operator_packages: Vec<SparkEncryptedOperatorPackage>,
    pub payment_hash: String,
}

pub(crate) const SPARK_PREPARE_TRANSFER_PATH: &str = "/public/v1/submit/spark_prepare_transfer";
pub(crate) const SPARK_PREPARE_TRANSFER_TYPE: &str = "ACTIVITY_TYPE_SPARK_PREPARE_TRANSFER";
pub(crate) const SPARK_PREPARE_TRANSFER_RESULT: &str = "sparkPrepareTransferResult";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkTransferLeaf {
    pub leaf_id: String,
    pub old_leaf_derivation: SparkKeyDerivation,
    pub new_leaf_derivation: SparkKeyDerivation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refund_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct_refund_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct_from_cpfp_refund_signature: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkTransferPackage {
    pub transfer_id: String,
    pub leaves: Vec<SparkTransferLeaf>,
    pub threshold: u32,
    pub operator_recipients: Vec<SparkOperatorRecipient>,
    pub receiver_public_key: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkPrepareTransferIntent {
    pub sign_with: String,
    pub transfer: SparkTransferPackage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkLeafPublicKey {
    pub leaf_id: String,
    pub public_key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkPrepareTransferResult {
    #[serde(default)]
    pub operator_packages: Vec<SparkEncryptedOperatorPackage>,
    pub transfer_user_signature: String,
    #[serde(default)]
    pub new_leaf_public_keys: Vec<SparkLeafPublicKey>,
}

pub(crate) const SPARK_CLAIM_TRANSFER_PATH: &str = "/public/v1/submit/spark_claim_transfer";
pub(crate) const SPARK_CLAIM_TRANSFER_TYPE: &str = "ACTIVITY_TYPE_SPARK_CLAIM_TRANSFER";
pub(crate) const SPARK_CLAIM_TRANSFER_RESULT: &str = "sparkClaimTransferResult";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkClaimLeaf {
    pub leaf_id: String,
    pub ciphertext: String,
    pub sender_signature: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkClaimPackage {
    pub leaves: Vec<SparkClaimLeaf>,
    pub threshold: u32,
    pub transfer_id: String,
    pub operator_recipients: Vec<SparkOperatorRecipient>,
    pub sender_identity_public_key: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkClaimTransferIntent {
    pub sign_with: String,
    pub claim: SparkClaimPackage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SparkClaimTransferResult {
    #[serde(default)]
    pub operator_packages: Vec<SparkEncryptedOperatorPackage>,
    #[serde(default)]
    pub new_leaf_public_keys: Vec<SparkLeafPublicKey>,
}

pub(crate) const EXPORT_WALLET_ACCOUNT_PATH: &str = "/public/v1/submit/export_wallet_account";
pub(crate) const EXPORT_WALLET_ACCOUNT_TYPE: &str = "ACTIVITY_TYPE_EXPORT_WALLET_ACCOUNT";
pub(crate) const EXPORT_WALLET_ACCOUNT_RESULT: &str = "exportWalletAccountResult";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportWalletAccountIntent {
    pub address: String,
    pub target_public_key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportWalletAccountResult {
    pub export_bundle: String,
}
