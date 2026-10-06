use std::{
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use breez_sdk_spark::{
    BreezSdk, CreateTokenAllowanceRequest, ListTokenAllowancesRequest, PaymentStatus,
    PreparePullPaymentResponse, PullPaymentRequest, PullPaymentResponse,
    RevokeTokenAllowanceRequest, SdkError, TokenAllowanceLimit, TokenAllowanceRole,
};
use clap::Subcommand;

use crate::command::print_value;

const PULL_RETRY_INTERVAL: Duration = Duration::from_secs(5);
const PULL_RETRY_WINDOW: Duration = Duration::from_mins(10);

#[derive(Clone, Debug)]
pub struct TokenAllowanceLimitArg(TokenAllowanceLimit);

impl FromStr for TokenAllowanceLimitArg {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("unlimited") {
            return Ok(Self(TokenAllowanceLimit::Unlimited));
        }
        let amount = s
            .parse::<u128>()
            .map_err(|_| format!("Invalid limit '{s}': expected an amount or 'unlimited'"))?;
        Ok(Self(TokenAllowanceLimit::Amount { amount }))
    }
}

impl From<TokenAllowanceLimitArg> for TokenAllowanceLimit {
    fn from(arg: TokenAllowanceLimitArg) -> Self {
        arg.0
    }
}

#[derive(Clone, Debug, Subcommand)]
pub enum TokenAllowanceCommand {
    /// Grant a spender an allowance over this wallet's tokens
    Create {
        /// Identity public key of the spender
        spender_public_key: String,
        /// Identifier of the token
        token_identifier: String,
        /// Expiry as a Unix timestamp in seconds
        expiry_time: u64,
        /// Limit on one payment, in token base units, or `unlimited`
        #[arg(long)]
        max_per_payment: TokenAllowanceLimitArg,
        /// Limit on all payments together, in token base units, or `unlimited`
        #[arg(long)]
        max_total: TokenAllowanceLimitArg,
        /// Identity public key the spender may pay. Repeat for each recipient, or omit for any.
        #[arg(short = 'r', long = "recipient")]
        allowed_recipients: Vec<String>,
    },

    /// Revoke a token allowance this wallet granted
    Revoke {
        /// Identifier of the allowance
        allowance_id: String,
    },

    /// List token allowances this wallet granted (owner) or received (spender)
    List {
        /// owner or spender
        #[arg(default_value = "owner")]
        role: TokenAllowanceRole,
        /// Only allowances with this counterparty identity public key
        #[arg(long)]
        counterparty_public_key: Option<String>,
        /// Only allowances for this token
        #[arg(long)]
        token_identifier: Option<String>,
        /// Include revoked, expired and exhausted allowances
        #[arg(long)]
        include_inactive: Option<bool>,
        /// Number of allowances to skip
        #[arg(long)]
        offset: Option<u32>,
        /// Maximum number of allowances to return
        #[arg(long)]
        limit: Option<u32>,
    },
}

pub async fn handle_command(
    sdk: &BreezSdk,
    command: TokenAllowanceCommand,
) -> Result<bool, anyhow::Error> {
    match command {
        TokenAllowanceCommand::Create {
            spender_public_key,
            token_identifier,
            expiry_time,
            max_per_payment,
            max_total,
            allowed_recipients,
        } => {
            let response = sdk
                .create_token_allowance(CreateTokenAllowanceRequest {
                    spender_public_key,
                    token_identifier,
                    max_per_payment: max_per_payment.into(),
                    max_total: max_total.into(),
                    expiry_time,
                    allowed_recipients,
                })
                .await?;
            print_value(&response.allowance)?;
            Ok(true)
        }
        TokenAllowanceCommand::Revoke { allowance_id } => {
            sdk.revoke_token_allowance(RevokeTokenAllowanceRequest { allowance_id })
                .await?;
            println!("Token allowance revoked");
            Ok(true)
        }
        TokenAllowanceCommand::List {
            role,
            counterparty_public_key,
            token_identifier,
            include_inactive,
            offset,
            limit,
        } => {
            let response = sdk
                .list_token_allowances(ListTokenAllowancesRequest {
                    role,
                    counterparty_public_key,
                    token_identifier,
                    include_inactive,
                    offset,
                    limit,
                })
                .await?;
            print_value(&response.allowances)?;
            Ok(true)
        }
    }
}

pub async fn pull_until_settled(
    sdk: &BreezSdk,
    prepare_response: PreparePullPaymentResponse,
) -> Result<PullPaymentResponse, SdkError> {
    let deadline =
        Duration::from_secs(prepare_response.expiry_time).saturating_add(PULL_RETRY_WINDOW);
    loop {
        let result = sdk
            .pull_payment(PullPaymentRequest {
                prepare_response: prepare_response.clone(),
            })
            .await;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        if !may_still_settle(&result) || now >= deadline {
            return result;
        }
        match &result {
            Ok(response) => println!(
                "The pull {} is pending, checking again in {}s",
                response.tx_hash,
                PULL_RETRY_INTERVAL.as_secs()
            ),
            Err(e) => println!(
                "Sending the pull failed ({e}), trying again in {}s",
                PULL_RETRY_INTERVAL.as_secs()
            ),
        }
        tokio::time::sleep(PULL_RETRY_INTERVAL).await;
    }
}

fn may_still_settle(result: &Result<PullPaymentResponse, SdkError>) -> bool {
    match result {
        Ok(response) => response.status == PaymentStatus::Pending,
        Err(error) => matches!(error, SdkError::NetworkError(_) | SdkError::SparkError(_)),
    }
}

#[cfg(test)]
mod tests {
    use breez_sdk_spark::{
        PaymentStatus, PullPaymentResponse, SdkError, TokenAllowanceErrorReason,
    };

    use super::may_still_settle;

    fn pulled(status: PaymentStatus) -> PullPaymentResponse {
        PullPaymentResponse {
            tx_hash: "pull".to_string(),
            status,
            payment: None,
        }
    }

    fn allowance(reason: TokenAllowanceErrorReason) -> SdkError {
        SdkError::TokenAllowance {
            reason,
            details: reason.to_string(),
        }
    }

    #[test]
    fn retries_only_while_the_pull_may_still_settle() {
        assert!(may_still_settle(&Ok(pulled(PaymentStatus::Pending))));
        assert!(!may_still_settle(&Ok(pulled(PaymentStatus::Completed))));
        assert!(may_still_settle(&Err(SdkError::NetworkError(
            "timeout".to_string()
        ))));
        assert!(may_still_settle(&Err(SdkError::SparkError(
            "transport error".to_string()
        ))));
        assert!(!may_still_settle(&Err(allowance(
            TokenAllowanceErrorReason::OverTotalLimit
        ))));
        assert!(!may_still_settle(&Err(allowance(
            TokenAllowanceErrorReason::Revoked
        ))));
        assert!(!may_still_settle(&Err(allowance(
            TokenAllowanceErrorReason::PreparedPullStale
        ))));
        assert!(!may_still_settle(&Err(SdkError::InvalidInput(
            "edited prepare response".to_string()
        ))));
        assert!(!may_still_settle(&Err(SdkError::Generic(
            "unexpected".to_string()
        ))));
    }
}
