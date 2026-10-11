use spark_wallet::{NewTokenAllowance, PublicKey, SparkAddress, TokenAllowanceQuery};
use tracing::error;

use crate::{
    CreateTokenAllowanceRequest, CreateTokenAllowanceResponse, ListTokenAllowancesRequest,
    ListTokenAllowancesResponse, Network, PaymentStatus, PreparePullPaymentRequest,
    PreparePullPaymentResponse, PullPaymentRequest, PullPaymentResponse, PullReceiver,
    RevokeTokenAllowanceRequest, TokenAllowance, error::SdkError, persist::ObjectCacheRepository,
    utils::token::token_transaction_to_payments,
};

use super::BreezSdk;

const DEFAULT_LIST_LIMIT: u32 = 100;

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    /// Grants a spender an allowance over this wallet's tokens.
    pub async fn create_token_allowance(
        &self,
        request: CreateTokenAllowanceRequest,
    ) -> Result<CreateTokenAllowanceResponse, SdkError> {
        let network = self.config.network;
        let allowed_recipients = request
            .allowed_recipients
            .iter()
            .map(|address| parse_address(address, network))
            .collect::<Result<Vec<_>, _>>()?;
        let allowance = self
            .spark_wallet
            .create_token_allowance(NewTokenAllowance {
                spender_public_key: parse_address(&request.spender_address, network)?,
                token_identifier: request.token_identifier,
                max_per_payment: request.max_per_payment.into(),
                max_total: request.max_total.into(),
                expiry_time: request.expiry_time,
                allowed_recipients,
            })
            .await?;
        Ok(CreateTokenAllowanceResponse {
            allowance: allowance_from(allowance, network)?,
        })
    }

    /// Revokes an allowance this wallet granted.
    pub async fn revoke_token_allowance(
        &self,
        request: RevokeTokenAllowanceRequest,
    ) -> Result<(), SdkError> {
        self.spark_wallet
            .revoke_token_allowance(&request.allowance_id)
            .await?;
        Ok(())
    }

    /// Lists allowances this wallet granted or received.
    pub async fn list_token_allowances(
        &self,
        request: ListTokenAllowancesRequest,
    ) -> Result<ListTokenAllowancesResponse, SdkError> {
        let network = self.config.network;
        let allowances = self
            .spark_wallet
            .query_token_allowances(TokenAllowanceQuery {
                role: request.role.into(),
                counterparty_public_key: request
                    .counterparty_address
                    .as_deref()
                    .map(|address| parse_address(address, network))
                    .transpose()?,
                token_identifier: request.token_identifier,
                include_inactive: request.include_inactive.unwrap_or(false),
                offset: request.offset.unwrap_or(0),
                limit: request.limit.unwrap_or(DEFAULT_LIST_LIMIT),
            })
            .await?;
        Ok(ListTokenAllowancesResponse {
            allowances: allowances
                .into_iter()
                .map(|allowance| allowance_from(allowance, network))
                .collect::<Result<_, _>>()?,
        })
    }

    /// Prepares a pull from a wallet that granted this wallet an allowance.
    pub async fn prepare_pull_payment(
        &self,
        request: PreparePullPaymentRequest,
    ) -> Result<PreparePullPaymentResponse, SdkError> {
        let network = self.config.network;
        let own_key = self.spark_wallet.get_identity_public_key();
        let receivers = request
            .receivers
            .iter()
            .map(|receiver| {
                Ok(spark_wallet::PullReceiver {
                    receiver_public_key: match &receiver.receiver_address {
                        Some(address) => parse_address(address, network)?,
                        None => own_key,
                    },
                    amount: receiver.amount,
                })
            })
            .collect::<Result<Vec<_>, SdkError>>()?;
        let prepared = self
            .spark_wallet
            .prepare_token_pull(
                parse_address(&request.payer_address, network)?,
                &request.token_identifier,
                receivers,
            )
            .await?;
        prepare_response_from(&prepared, network)
    }

    /// Signs and broadcasts a prepared pull, and can be retried with the same prepare response.
    pub async fn pull_payment(
        &self,
        request: PullPaymentRequest,
    ) -> Result<PullPaymentResponse, SdkError> {
        let prepared = decode_prepare_response(&request.prepare_response, self.config.network)?;
        self.maybe_ensure_spark_private_mode_initialized().await?;
        let transaction = self
            .spark_wallet
            .sign_and_broadcast_token_pull(&prepared)
            .await?;
        Ok(self.complete_pull(transaction).await)
    }
}

impl BreezSdk {
    pub(in crate::sdk) async fn complete_pull(
        &self,
        transaction: spark_wallet::TokenTransaction,
    ) -> PullPaymentResponse {
        if let Some(pending) = pending_pull(&transaction) {
            return pending;
        }
        let object_repository = ObjectCacheRepository::new(self.storage.clone());
        let payments = match token_transaction_to_payments(
            &self.spark_wallet,
            &object_repository,
            &transaction,
            false,
        )
        .await
        {
            Ok(payments) => payments,
            Err(e) => {
                error!("Failed to map settled pull {}: {e:?}", transaction.hash);
                return PullPaymentResponse {
                    tx_hash: transaction.hash,
                    status: PaymentStatus::Completed,
                    payment: None,
                };
            }
        };
        let mut new_payments = Vec::new();
        for payment in &payments {
            match self.storage.apply_payment_update(payment.clone()).await {
                Ok(true) => new_payments.push(payment.clone()),
                Ok(false) => {}
                Err(e) => error!("Failed to store pulled payment {}: {e:?}", payment.id),
            }
        }
        if !new_payments.is_empty()
            && let Err(e) = self
                .spark_wallet
                .process_token_transaction(&transaction)
                .await
        {
            error!(
                "Failed to add the outputs of pull {} to the wallet: {e:?}",
                transaction.hash
            );
        }
        super::payments::send::emit_payments(self, &new_payments).await;
        PullPaymentResponse {
            tx_hash: transaction.hash,
            status: PaymentStatus::Completed,
            payment: payments.into_iter().next(),
        }
    }
}

fn pending_pull(transaction: &spark_wallet::TokenTransaction) -> Option<PullPaymentResponse> {
    (!matches!(
        transaction.status,
        spark_wallet::TokenTransactionStatus::Finalized
    ))
    .then(|| PullPaymentResponse {
        tx_hash: transaction.hash.clone(),
        status: PaymentStatus::Pending,
        payment: None,
    })
}

fn parse_address(address: &str, network: Network) -> Result<PublicKey, SdkError> {
    let parsed = address
        .parse::<SparkAddress>()
        .map_err(|_| SdkError::InvalidInput(format!("Invalid spark address: {address}")))?;
    if parsed.spark_invoice_fields.is_some() {
        return Err(SdkError::InvalidInput(format!(
            "Expected a spark address, not an invoice: {address}"
        )));
    }
    if parsed.network != network.into() {
        return Err(SdkError::InvalidInput(format!(
            "The spark address is for another network: {address}"
        )));
    }
    Ok(parsed.identity_public_key)
}

pub(in crate::sdk) fn address_of(key: PublicKey, network: Network) -> Result<String, SdkError> {
    SparkAddress::new(key, network.into(), None)
        .to_address_string()
        .map_err(|e| SdkError::Generic(format!("Failed to encode a spark address: {e}")))
}

fn allowance_from(
    allowance: spark_wallet::TokenAllowance,
    network: Network,
) -> Result<TokenAllowance, SdkError> {
    Ok(TokenAllowance {
        id: allowance.id,
        owner_address: address_of(allowance.owner_public_key, network)?,
        spender_address: address_of(allowance.spender_public_key, network)?,
        token_identifier: allowance.token_identifier,
        max_per_payment: allowance.max_per_payment.into(),
        max_total: allowance.max_total.into(),
        spent_amount: allowance.spent_amount,
        allowed_recipients: allowance
            .allowed_recipients
            .into_iter()
            .map(|key| address_of(key, network))
            .collect::<Result<_, _>>()?,
        expiry_time: allowance.expiry_time,
        created_at: allowance.created_at,
        revoked_at: allowance.revoked_at,
        status: allowance.status.into(),
    })
}

pub(in crate::sdk) fn encode_pull_context(
    prepared: &spark_wallet::PreparedTokenPull,
) -> Result<Vec<u8>, SdkError> {
    serde_json::to_vec(prepared)
        .map_err(|e| SdkError::Generic(format!("Failed to serialize the pull: {e}")))
}

pub(in crate::sdk) fn decode_pull_context(
    bytes: &[u8],
) -> Result<spark_wallet::PreparedTokenPull, SdkError> {
    serde_json::from_slice(bytes)
        .map_err(|e| SdkError::InvalidInput(format!("Invalid pull context: {e}")))
}

pub(in crate::sdk) fn decode_prepare_response(
    response: &PreparePullPaymentResponse,
    network: Network,
) -> Result<spark_wallet::PreparedTokenPull, SdkError> {
    let prepared = decode_pull_context(&response.pull_context)?;
    if response.allowance_id != prepared.allowance_id
        || response.expiry_time != prepared.expiry_time()
        || !describes_pull(
            &prepared,
            network,
            &response.payer_address,
            &response.token_identifier,
            &response.receivers,
            response.amount,
        )
    {
        return Err(SdkError::InvalidInput(
            "The prepare response doesn't match its pull context".to_string(),
        ));
    }
    Ok(prepared)
}

pub(in crate::sdk) fn describes_pull(
    prepared: &spark_wallet::PreparedTokenPull,
    network: Network,
    payer_address: &str,
    token_identifier: &str,
    receivers: &[PullReceiver],
    amount: u128,
) -> bool {
    address_of(prepared.payer_public_key, network).is_ok_and(|address| address == payer_address)
        && token_identifier == prepared.token_identifier
        && pull_receivers(prepared, network).is_ok_and(|expected| expected == receivers)
        && amount == prepared.total()
}

pub(in crate::sdk) fn pull_receivers(
    prepared: &spark_wallet::PreparedTokenPull,
    network: Network,
) -> Result<Vec<PullReceiver>, SdkError> {
    prepared
        .receivers
        .iter()
        .map(|receiver| {
            Ok(PullReceiver {
                amount: receiver.amount,
                receiver_address: Some(address_of(receiver.receiver_public_key, network)?),
            })
        })
        .collect()
}

fn prepare_response_from(
    prepared: &spark_wallet::PreparedTokenPull,
    network: Network,
) -> Result<PreparePullPaymentResponse, SdkError> {
    Ok(PreparePullPaymentResponse {
        payer_address: address_of(prepared.payer_public_key, network)?,
        token_identifier: prepared.token_identifier.clone(),
        receivers: pull_receivers(prepared, network)?,
        amount: prepared.total(),
        allowance_id: prepared.allowance_id.clone(),
        expiry_time: prepared.expiry_time(),
        pull_context: encode_pull_context(prepared)?,
    })
}

#[cfg(test)]
pub(crate) fn prepare_response_for_tests(
    prepared: &spark_wallet::PreparedTokenPull,
) -> Result<PreparePullPaymentResponse, SdkError> {
    prepare_response_from(prepared, Network::Regtest)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::str::FromStr;

    use platform_utils::time::SystemTime;
    use spark_wallet::{
        PreparedTokenPull, PublicKey, PullReceiver, TokenInputs, TokenTransaction,
        TokenTransactionStatus, TokenTransferInput,
    };

    use super::{
        address_of, decode_prepare_response, decode_pull_context, encode_pull_context,
        pending_pull, prepare_response_from,
    };
    use crate::{Network, PaymentStatus, PreparePullPaymentResponse, error::SdkError};

    pub(crate) const OTHER_KEY: &str =
        "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

    pub(crate) fn other_address() -> String {
        address_of(PublicKey::from_str(OTHER_KEY).unwrap(), Network::Regtest).unwrap()
    }

    pub(crate) fn sample_pull() -> PreparedTokenPull {
        let key = PublicKey::from_str(
            "02ca75659458529755b77663f18282f4aa130313e098fac40deffb1208207a2ffe",
        )
        .unwrap();
        PreparedTokenPull {
            allowance_id: uuid::Uuid::now_v7().to_string(),
            payer_public_key: key,
            token_identifier: "btknrt1example".to_string(),
            receivers: vec![PullReceiver {
                receiver_public_key: key,
                amount: 42,
            }],
            partial_token_transaction_bytes: vec![1, 2, 3],
            partial_token_transaction_hash: vec![7; 32],
            created_timestamp: SystemTime::now(),
            allowance_expiry_time: 4_102_444_800,
        }
    }

    #[test]
    fn pull_context_round_trips() {
        let pull = sample_pull();
        let decoded = decode_pull_context(&encode_pull_context(&pull).unwrap()).unwrap();
        assert_eq!(decoded.allowance_id, pull.allowance_id);
        assert_eq!(
            decoded.partial_token_transaction_hash,
            pull.partial_token_transaction_hash
        );
    }

    #[test]
    fn prepare_response_reports_total_and_window() {
        let pull = sample_pull();
        let response = prepare_response_from(&pull, Network::Regtest).unwrap();
        assert_eq!(response.amount, 42);
        assert_eq!(response.allowance_id, pull.allowance_id);
        assert_eq!(response.expiry_time, pull.expiry_time());
        let address = address_of(pull.receivers[0].receiver_public_key, Network::Regtest).unwrap();
        assert!(address.starts_with("sparkrt1"));
        assert_eq!(response.receivers[0].receiver_address, Some(address));
    }

    #[test]
    fn prepare_response_reports_an_earlier_allowance_expiry() {
        let pull = PreparedTokenPull {
            allowance_expiry_time: 1_000,
            ..sample_pull()
        };
        assert_eq!(
            prepare_response_from(&pull, Network::Regtest)
                .unwrap()
                .expiry_time,
            1_000
        );
    }

    #[test]
    fn rejects_garbage_context() {
        assert!(decode_pull_context(b"not json").is_err());
    }

    #[test]
    fn a_pull_that_is_not_final_is_pending() {
        let pull = |status| TokenTransaction {
            hash: "pull".to_string(),
            inputs: TokenInputs::Transfer(TokenTransferInput {
                outputs_to_spend: vec![],
            }),
            outputs: vec![],
            status,
            created_timestamp: SystemTime::now(),
            fulfilled_invoices: vec![],
        };
        for status in [
            TokenTransactionStatus::Unknown,
            TokenTransactionStatus::Signed,
            TokenTransactionStatus::Revealed,
        ] {
            let response = pending_pull(&pull(status)).unwrap();
            assert_eq!(response.tx_hash, "pull");
            assert_eq!(response.status, PaymentStatus::Pending);
            assert!(response.payment.is_none());
        }
        assert!(pending_pull(&pull(TokenTransactionStatus::Finalized)).is_none());
    }

    #[test]
    fn accepts_an_unchanged_prepare_response() {
        let pull = sample_pull();
        let response = prepare_response_from(&pull, Network::Regtest).unwrap();
        let decoded = decode_prepare_response(&response, Network::Regtest).unwrap();
        assert_eq!(decoded.allowance_id, pull.allowance_id);
        assert_eq!(decoded.receivers, pull.receivers);
        assert!(matches!(
            decode_prepare_response(&response, Network::Mainnet),
            Err(SdkError::InvalidInput(_))
        ));
    }

    #[test]
    fn refuses_a_prepare_response_edited_after_prepare() {
        let mut pull = sample_pull();
        pull.receivers.push(PullReceiver {
            receiver_public_key: PublicKey::from_str(OTHER_KEY).unwrap(),
            amount: 8,
        });
        let original = prepare_response_from(&pull, Network::Regtest).unwrap();
        let edits: [fn(&mut PreparePullPaymentResponse); 10] = [
            |r| r.payer_address = other_address(),
            |r| r.token_identifier = "btknrt1other".to_string(),
            |r| r.receivers[0].receiver_address = Some(other_address()),
            |r| r.receivers[0].receiver_address = None,
            |r| r.receivers[0].amount += 1,
            |r| r.receivers.swap(0, 1),
            |r| r.receivers.truncate(1),
            |r| r.amount += 1,
            |r| r.allowance_id = uuid::Uuid::now_v7().to_string(),
            |r| r.expiry_time += 1,
        ];
        for edit in edits {
            let mut response = original.clone();
            edit(&mut response);
            assert!(matches!(
                decode_prepare_response(&response, Network::Regtest),
                Err(SdkError::InvalidInput(_))
            ));
        }
    }
}
