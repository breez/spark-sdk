#![allow(
    clippy::unused_async,
    clippy::wildcard_imports,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::arithmetic_side_effects
)]

use std::str::FromStr;
use std::sync::Arc;

use async_graphql::*;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::require_auth;
use super::scalars::Long;
#[allow(clippy::wildcard_imports)]
use super::types::*;
use crate::auth::AuthService;
use crate::coop_exit::CoopExitService;
use crate::lightning::node::InvoiceDescription;
use crate::static_deposit::StaticDepositService;
use crate::swap::SwapService;

pub struct MutationRoot;

pub(super) fn currency_amount_sats(sats: u64) -> CurrencyAmount {
    CurrencyAmount {
        original_value: Long(sats as i64),
        original_unit: CurrencyUnit::Satoshi,
        preferred_currency_unit: CurrencyUnit::Satoshi,
        preferred_currency_value_rounded: Long(sats as i64),
        preferred_currency_value_approx: sats as f64,
    }
}

#[Object(rename_fields = "snake_case", rename_args = "snake_case")]
impl MutationRoot {
    async fn claim_static_deposit(
        &self,
        ctx: &Context<'_>,
        input: ClaimStaticDepositInput,
    ) -> Result<ClaimStaticDepositOutput> {
        // The caller is credited, and the operators credit no one but the deposit's owner.
        let caller = require_auth(ctx)?;
        let service = ctx.data::<Arc<StaticDepositService>>()?;
        let transfer_id = service
            .claim(&input, caller)
            .await
            .map_err(|e| Error::new(format!("claim static deposit failed: {e}")))?;
        let transfer_id = Uuid::parse_str(&transfer_id)
            .map_err(|e| Error::new(format!("invalid transfer id: {e}")))?;
        Ok(ClaimStaticDepositOutput { transfer_id })
    }

    async fn complete_coop_exit(
        &self,
        ctx: &Context<'_>,
        input: CompleteCoopExitInput,
    ) -> Result<CompleteCoopExitOutput> {
        let caller = require_auth(ctx)?;
        let service = ctx.data::<Arc<CoopExitService>>()?;
        let network = *ctx.data::<BitcoinNetwork>()?;
        let request_id = input
            .coop_exit_request_id
            .as_ref()
            .ok_or_else(|| Error::new("coop_exit_request_id is required"))?
            .to_string();
        let record = service
            .complete_coop_exit(&request_id, caller)
            .await
            .map_err(|e| Error::new(format!("complete coop exit failed: {e}")))?;
        Ok(CompleteCoopExitOutput {
            request: super::coop_exit::coop_exit_record_to_type(&record, network),
        })
    }

    #[graphql(complexity = "super::EXTERNAL_CALL_COMPLEXITY + child_complexity")]
    async fn create_instant_static_deposit_quote(
        &self,
        ctx: &Context<'_>,
        input: CreateInstantStaticDepositQuoteInput,
    ) -> Result<CreateInstantStaticDepositQuoteOutput> {
        require_auth(ctx)?;
        let service = ctx.data::<Arc<StaticDepositService>>()?;
        let txid = bitcoin::Txid::from_str(&input.transaction_id)
            .map_err(|e| Error::new(format!("invalid transaction_id: {e}")))?;
        let vout = u32::try_from(input.output_index)
            .map_err(|_| Error::new("output_index must be non-negative"))?;
        let quote = service
            .instant_quote(&txid, vout, spark::Network::from(input.network))
            .await
            .map_err(|e| Error::new(format!("instant static deposit quote failed: {e}")))?;
        Ok(CreateInstantStaticDepositQuoteOutput {
            quote: InstantStaticDepositQuote {
                id: ID::from(quote.id.clone()),
                transaction_id: quote.txid,
                output_index: input.output_index,
                deposit_amount: currency_amount_sats(quote.deposit_amount_sats),
                credit_amount: currency_amount_sats(quote.credit_amount_sats),
                quote_signature: quote.quote_signature,
            },
            // One 0-conf plan: the quoted credit is paid in full when the claim is reserved.
            fulfillment_plans: vec![StaticDepositPlan {
                id: ID::from(quote.id),
                amount: currency_amount_sats(quote.credit_amount_sats),
                confirmations: 0,
            }],
        })
    }

    async fn create_claim_instant_static_deposit(
        &self,
        ctx: &Context<'_>,
        input: CreateClaimInstantStaticDepositInput,
    ) -> Result<CreateClaimInstantStaticDepositOutput> {
        // The caller is credited, and the operators credit no one but the deposit's owner.
        let caller = require_auth(ctx)?;
        let service = ctx.data::<Arc<StaticDepositService>>()?;
        let claim_id = service
            .claim_instant(&input, caller)
            .await
            .map_err(|e| Error::new(format!("claim instant static deposit failed: {e}")))?;
        let claim_id =
            Uuid::parse_str(&claim_id).map_err(|e| Error::new(format!("invalid claim id: {e}")))?;
        Ok(CreateClaimInstantStaticDepositOutput { claim_id })
    }

    async fn get_challenge(
        &self,
        ctx: &Context<'_>,
        input: GetChallengeInput,
    ) -> Result<GetChallengeOutput> {
        let auth = ctx.data::<Arc<AuthService>>()?;
        let protected_challenge = auth
            .issue_challenge(&input.public_key.0, Utc::now().timestamp())
            .map_err(|e| Error::new(format!("failed to issue challenge: {e}")))?;
        Ok(GetChallengeOutput {
            protected_challenge,
        })
    }

    async fn request_coop_exit(
        &self,
        ctx: &Context<'_>,
        input: RequestCoopExitInput,
    ) -> Result<RequestCoopExitOutput> {
        let caller = require_auth(ctx)?;
        let service = ctx.data::<Arc<CoopExitService>>()?;
        let network = *ctx.data::<BitcoinNetwork>()?;
        let record = service
            .request_coop_exit(&input, caller)
            .await
            .map_err(|e| Error::new(format!("coop exit failed: {e}")))?;
        Ok(RequestCoopExitOutput {
            request: super::coop_exit::coop_exit_record_to_type(&record, network),
        })
    }

    async fn request_lightning_send(
        &self,
        ctx: &Context<'_>,
        input: RequestLightningSendInput,
    ) -> Result<RequestLightningSendOutput> {
        let caller = require_auth(ctx)?;
        let service = &super::Lightning::services(ctx)?.send;
        let network = *ctx.data::<BitcoinNetwork>()?;
        let user_transfer_id = input
            .user_outbound_transfer_external_id
            .ok_or_else(|| Error::new("user_outbound_transfer_external_id is required"))?
            .to_string();
        let amount_sats = input.amount_sats.and_then(|a| u64::try_from(a.0).ok());
        let record = service
            .request_lightning_send(
                &caller,
                &input.encoded_invoice,
                amount_sats,
                input.idempotency_key.as_deref(),
                &user_transfer_id,
            )
            .await
            .map_err(|e| Error::new(format!("lightning send failed: {e}")))?;
        Ok(RequestLightningSendOutput {
            request: super::lightning::send_record_to_type(&record, network),
        })
    }

    async fn request_lightning_receive(
        &self,
        ctx: &Context<'_>,
        input: RequestLightningReceiveInput,
    ) -> Result<RequestLightningReceiveOutput> {
        let user_pubkey = require_auth(ctx)?;
        let service = &super::Lightning::services(ctx)?.receive;
        let network = *ctx.data::<BitcoinNetwork>()?;
        let payment_hash: [u8; 32] = hex::decode(&input.payment_hash.0)
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(|| Error::new("payment_hash must be 32 bytes of hex"))?;
        // The receiver need not be the caller: an LNURL server requests invoices for its users.
        let receiver_pubkey = match &input.receiver_identity_pubkey {
            Some(receiver) => bitcoin::secp256k1::PublicKey::from_str(&receiver.0)
                .map_err(|e| Error::new(format!("invalid receiver_identity_pubkey: {e}")))?,
            None => user_pubkey,
        };
        if input.spark_invoice.is_some() {
            return Err(Error::new("spark_invoice is not supported by this SSP"));
        }
        let amount_sats = u64::try_from(input.amount_sats.0)
            .map_err(|_| Error::new("amount_sats must not be negative"))?;
        let expiry_secs = input
            .expiry_secs
            .map_or(Ok(3600), u32::try_from)
            .map_err(|_| Error::new("expiry_secs must not be negative"))?;
        let description = match (input.memo, input.description_hash) {
            (Some(_), Some(_)) => {
                return Err(Error::new(
                    "memo and description_hash are mutually exclusive",
                ));
            }
            (_, Some(hash)) => {
                let bytes: [u8; 32] = hex::decode(&hash.0)
                    .ok()
                    .and_then(|b| b.try_into().ok())
                    .ok_or_else(|| Error::new("description_hash must be 32 bytes of hex"))?;
                InvoiceDescription::Hash(bytes)
            }
            (Some(memo), None) => InvoiceDescription::Memo(memo),
            (None, None) => InvoiceDescription::Memo(String::new()),
        };
        let record = service
            .request_lightning_receive(
                payment_hash,
                amount_sats,
                &user_pubkey,
                &receiver_pubkey,
                &description,
                expiry_secs,
                input.include_spark_address,
            )
            .await
            .map_err(|e| Error::new(format!("lightning receive failed: {e}")))?;
        Ok(RequestLightningReceiveOutput {
            request: super::lightning::receive_record_to_type(&record, network),
        })
    }

    async fn request_swap(
        &self,
        ctx: &Context<'_>,
        input: RequestSwapInput,
    ) -> Result<RequestSwapOutput> {
        let caller = require_auth(ctx)?;
        let swap_service = ctx.data::<Arc<SwapService>>()?;
        let network = *ctx.data::<BitcoinNetwork>()?;
        let result = swap_service
            .request_swap(&input, caller)
            .await
            .map_err(|e| Error::new(format!("swap failed: {e}")))?;

        let transfer = &result.counter_transfer;
        let transfer_id = transfer.id.clone();
        let leaves: Vec<Leaf> = transfer
            .leaves
            .iter()
            .map(|leaf| Leaf {
                amount: currency_amount_sats(leaf.leaf.value),
                spark_node_id: Uuid::parse_str(&leaf.leaf.id.to_string()).unwrap_or_default(),
            })
            .collect();

        let now = Utc::now();
        let request = LeavesSwapRequestType {
            id: ID::from(result.swap_id.clone()),
            created_at: now,
            updated_at: now,
            network,
            request_status: Some(SparkUserRequestStatus::InProgress),
            status: SparkLeavesSwapRequestStatus::OutboundTransferSent,
            total_amount: currency_amount_sats(result.total_amount),
            target_amount: currency_amount_sats(result.target_amount),
            fee: currency_amount_sats(result.fee),
            inbound_transfer: Some(Transfer {
                total_amount: currency_amount_sats(result.target_amount),
                spark_id: Some(Uuid::parse_str(&transfer_id.to_string()).unwrap_or_default()),
                leaves: SparkTransferToLeavesConnection {
                    count: leaves.len() as i32,
                    page_info: PageInfo {
                        has_next_page: Some(false),
                        has_previous_page: Some(false),
                        start_cursor: None,
                        end_cursor: None,
                    },
                    entities: leaves,
                },
                user_request: None,
            }),
            outbound_transfer: None,
            expires_at: None,
            swap_leaves: None,
        };

        Ok(RequestSwapOutput { request })
    }

    async fn request_regtest_funds(
        &self,
        ctx: &Context<'_>,
        input: RequestRegtestFundsInput,
    ) -> Result<RequestRegtestFundsOutput> {
        let funder = ctx
            .data::<Option<Arc<dyn super::RegtestFunder>>>()?
            .as_ref()
            .ok_or_else(|| Error::new("regtest funds are not available on this network"))?;
        let amount_sats = u64::try_from(input.amount_sats.0)
            .map_err(|_| Error::new("amount_sats must be non-negative"))?;
        let transaction_hash = funder
            .send_to_address(&input.address, amount_sats)
            .await
            .map_err(|e| Error::new(format!("faucet send failed: {e}")))?;
        Ok(RequestRegtestFundsOutput { transaction_hash })
    }

    async fn verify_challenge(
        &self,
        ctx: &Context<'_>,
        input: VerifyChallengeInput,
    ) -> Result<VerifyChallengeOutput> {
        let auth = ctx.data::<Arc<AuthService>>()?;
        let (session_token, valid_until) = auth
            .verify_challenge(
                &input.protected_challenge,
                &input.signature,
                &input.identity_public_key.0,
                Utc::now().timestamp(),
            )
            .map_err(|e| Error::new(format!("challenge verification failed: {e}")))?;
        let valid_until = DateTime::<Utc>::from_timestamp(valid_until, 0)
            .ok_or_else(|| Error::new("invalid session expiry timestamp"))?;
        Ok(VerifyChallengeOutput {
            valid_until,
            session_token,
        })
    }

    async fn register_wallet_webhook(
        &self,
        _ctx: &Context<'_>,
        input: RegisterSparkWalletWebhookInput,
    ) -> Result<RegisterSparkWalletWebhookOutput> {
        let _ = input;
        Err("not implemented".into())
    }

    async fn delete_wallet_webhook(
        &self,
        _ctx: &Context<'_>,
        input: DeleteSparkWalletWebhookInput,
    ) -> Result<DeleteSparkWalletWebhookOutput> {
        let _ = input;
        Err("not implemented".into())
    }
}
