//! Typed request/result structs for the Turnkey activities and queries the
//! signer uses. JSON keys are camelCase; enum values are Turnkey's
//! SCREAMING_SNAKE strings.

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
}
