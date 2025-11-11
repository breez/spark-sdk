mod sdk;

pub use sdk::BreezIssuerSdk;
use serde::Serialize;

#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
#[derive(Debug, Serialize)]
pub struct CreateIssuerTokenRequest {
    pub name: String,
    pub ticker: String,
    pub decimals: u32,
    pub is_freezable: bool,
    pub max_supply: u128,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct GetIssuerTokenBalanceResponse {
    pub identifier: String,
    pub balance: u128,
}

impl From<spark_wallet::IssuerTokenBalance> for GetIssuerTokenBalanceResponse {
    fn from(value: spark_wallet::IssuerTokenBalance) -> Self {
        Self {
            identifier: value.identifier,
            balance: value.balance,
        }
    }
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MintIssuerTokenRequest {
    pub amount: u128,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BurnIssuerTokenRequest {
    pub amount: u128,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FreezeIssuerTokenRequest {
    pub address: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FreezeIssuerTokenResponse {
    pub impacted_output_ids: Vec<String>,
    pub impacted_token_amount: u128,
}

impl From<spark_wallet::FreezeIssuerTokenResponse> for FreezeIssuerTokenResponse {
    fn from(value: spark_wallet::FreezeIssuerTokenResponse) -> Self {
        Self {
            impacted_output_ids: value.impacted_output_ids,
            impacted_token_amount: value.impacted_token_amount,
        }
    }
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UnfreezeIssuerTokenRequest {
    pub address: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UnfreezeIssuerTokenResponse {
    pub impacted_output_ids: Vec<String>,
    pub impacted_token_amount: u128,
}

impl From<spark_wallet::FreezeIssuerTokenResponse> for UnfreezeIssuerTokenResponse {
    fn from(value: spark_wallet::FreezeIssuerTokenResponse) -> Self {
        Self {
            impacted_output_ids: value.impacted_output_ids,
            impacted_token_amount: value.impacted_token_amount,
        }
    }
}
