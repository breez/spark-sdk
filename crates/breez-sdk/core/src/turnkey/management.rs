//! Turnkey wallet lifecycle helpers (create / delete / list), behind the
//! `test-utils` feature.
//!
//! Not part of the integrator-facing API: integrators bring their own wallet
//! (see [`super::create_turnkey_signer`]). These exist so a test harness can
//! provision a throwaway wallet per test and reap abandoned ones, keeping each
//! test isolated. `test-utils` is the crate's cross-crate equivalent of
//! `cfg(test)`: the integration-test crate consumes this from outside, where a
//! literal `cfg(test)` would not be visible. The configured API key
//! authenticates at the organization level, so one key can create and delete
//! many wallets in that org.

use serde::{Deserialize, Serialize};

use super::accounts::spark_address_format;
use super::config::TurnkeyConfig;
use super::error::TurnkeyError;
use super::transport::{OnConflict, TurnkeyClient};
use super::types::{
    ADDRESS_FORMAT_COMPRESSED, CURVE_SECP256K1, PATH_FORMAT_BIP32, WalletAccountParams,
};

const CREATE_WALLET_PATH: &str = "/public/v1/submit/create_wallet";
const CREATE_WALLET_TYPE: &str = "ACTIVITY_TYPE_CREATE_WALLET";
const CREATE_WALLET_RESULT: &str = "createWalletResult";

const DELETE_WALLETS_PATH: &str = "/public/v1/submit/delete_wallets";
const DELETE_WALLETS_TYPE: &str = "ACTIVITY_TYPE_DELETE_WALLETS";
const DELETE_WALLETS_RESULT: &str = "deleteWalletsResult";

const LIST_WALLETS_PATH: &str = "/public/v1/query/list_wallets";

/// Identity derivation path under the configured account. A new wallet is
/// seeded with both the compressed and Spark identity accounts at this path
/// (see [`TurnkeyWalletManager::create_wallet`]).
fn identity_path(account: u32) -> String {
    format!("m/8797555'/{account}'/0'")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateWalletIntent {
    wallet_name: String,
    accounts: Vec<WalletAccountParams>,
    mnemonic_length: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateWalletResult {
    wallet_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteWalletsIntent {
    wallet_ids: Vec<String>,
    delete_without_export: bool,
}

// The result echoes the deleted ids; we don't need them, so deserialize into an
// empty struct (serde ignores the extra fields).
#[derive(Deserialize)]
struct DeleteWalletsResult {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListWalletsRequest {
    organization_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListWalletsResponse {
    #[serde(default)]
    wallets: Vec<WalletMetadata>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WalletMetadata {
    wallet_id: String,
    #[serde(default)]
    wallet_name: String,
    created_at: TurnkeyTimestamp,
}

#[derive(Deserialize)]
struct TurnkeyTimestamp {
    /// Whole seconds since the Unix epoch, as a decimal string.
    seconds: String,
}

/// A wallet in the organization, as returned by [`TurnkeyWalletManager::list_wallets`].
pub struct TurnkeyWalletInfo {
    pub wallet_id: String,
    pub wallet_name: String,
    /// Creation time in whole seconds since the Unix epoch (0 if unparseable).
    pub created_at_secs: u64,
}

/// Organization-scoped wallet lifecycle operations, reusing the signer's Turnkey
/// client (stamping, transport, retry).
pub struct TurnkeyWalletManager {
    client: TurnkeyClient,
    network: crate::Network,
    account: u32,
}

impl TurnkeyWalletManager {
    /// Builds a manager from `config`. Its `wallet_id` is unused: these calls are
    /// organization-scoped. Uses the default platform HTTP client.
    pub fn new(config: &TurnkeyConfig) -> Result<Self, TurnkeyError> {
        let http = platform_utils::create_http_client(Some("breez-sdk-spark-turnkey-test"));
        Ok(Self {
            client: TurnkeyClient::new(config, http)?,
            network: config.network,
            account: super::factory::account_number(config),
        })
    }

    /// Creates a fresh HD wallet (random seed) named `wallet_name`, ready for
    /// `create_turnkey_signer`, and returns the new wallet id.
    ///
    /// Seeds BOTH identity-account formats up front, in this one activity: the
    /// compressed key (ECDSA, for operator auth and user signatures) and the
    /// Spark key (BIP-340 Schnorr, for token and Spark-invoice signing). A
    /// second format cannot be added to an occupied path by a later call, and
    /// generic `SIGN_RAW` needs the account row to exist, so both must be
    /// materialized together here.
    pub async fn create_wallet(&self, wallet_name: String) -> Result<String, TurnkeyError> {
        let identity_account = |address_format: &'static str| WalletAccountParams {
            curve: CURVE_SECP256K1,
            path_format: PATH_FORMAT_BIP32,
            path: identity_path(self.account),
            address_format,
        };
        let intent = CreateWalletIntent {
            wallet_name: wallet_name.clone(),
            accounts: vec![
                identity_account(ADDRESS_FORMAT_COMPRESSED),
                identity_account(spark_address_format(self.network)),
            ],
            mnemonic_length: 24,
        };
        let result: Result<CreateWalletResult, TurnkeyError> = self
            .client
            .submit_activity(
                CREATE_WALLET_PATH,
                CREATE_WALLET_TYPE,
                intent,
                CREATE_WALLET_RESULT,
                // The 409 is recovered by name below, not retried.
                OnConflict::Surface,
            )
            .await;
        match result {
            Ok(result) => Ok(result.wallet_id),
            // A retried submit hits Turnkey's fingerprint dedup (409). The
            // original already created the wallet, so recover its id by the
            // unique name.
            Err(TurnkeyError::Http { status: 409, .. }) => self
                .list_wallets()
                .await?
                .into_iter()
                .find(|w| w.wallet_name == wallet_name)
                .map(|w| w.wallet_id)
                .ok_or_else(|| {
                    TurnkeyError::UnexpectedResponse(format!(
                        "create_wallet returned 409 but wallet '{wallet_name}' was not found"
                    ))
                }),
            Err(e) => Err(e),
        }
    }

    /// Deletes the given wallets without requiring a prior export. Deleting by
    /// wallet id only ever affects wallets the caller created, so concurrent
    /// runners never delete each other's wallets. A no-op for an empty list.
    pub async fn delete_wallets(&self, wallet_ids: Vec<String>) -> Result<(), TurnkeyError> {
        if wallet_ids.is_empty() {
            return Ok(());
        }
        let _: DeleteWalletsResult = self
            .client
            .submit_activity(
                DELETE_WALLETS_PATH,
                DELETE_WALLETS_TYPE,
                DeleteWalletsIntent {
                    wallet_ids,
                    delete_without_export: true,
                },
                DELETE_WALLETS_RESULT,
                OnConflict::Retry,
            )
            .await?;
        Ok(())
    }

    /// Lists every wallet in the organization with its creation time, so a caller
    /// can age-gate cleanup of abandoned test wallets.
    pub async fn list_wallets(&self) -> Result<Vec<TurnkeyWalletInfo>, TurnkeyError> {
        let request = ListWalletsRequest {
            organization_id: self.client.organization_id.clone(),
        };
        let response: ListWalletsResponse = self
            .client
            .process_request(LIST_WALLETS_PATH, &request, OnConflict::Surface)
            .await?;
        Ok(response
            .wallets
            .into_iter()
            .map(|w| TurnkeyWalletInfo {
                wallet_id: w.wallet_id,
                wallet_name: w.wallet_name,
                created_at_secs: w.created_at.seconds.parse().unwrap_or(0),
            })
            .collect())
    }
}
