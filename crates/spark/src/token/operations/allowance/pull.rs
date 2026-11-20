use std::collections::HashSet;

use bitcoin::secp256k1::{PublicKey, schnorr};
use platform_utils::time::SystemTime;
use prost::Message as _;
use tonic::{Code, Status};
use tonic_types::StatusExt;
use tracing::warn;
use uuid::Uuid;

use crate::{
    Network,
    address::SparkAddress,
    operator::rpc::{
        QueryAllTokenOutputsRequest,
        multisig::KeyedSignature,
        spark_token::{
            AllowanceSignature, BroadcastTransactionRequest, BroadcastTransactionResponse,
            CommitStatus, OutputWithPreviousTransactionData, PartialTokenTransaction,
            QueryTokenAllowancesRequest, SignatureWithIndex, TokenAllowanceInfo,
            TokenAllowanceStatus as ProtoAllowanceStatus, TokenOutputStatus,
            partial_token_transaction::TokenInputs, signature_with_index::AuthoritySignatures,
        },
    },
    services::{ServiceError, TokenTransaction, TokenTransactionStatus},
    signer::TokenTransactionKind,
    token::{
        MAX_TOKEN_TX_OUTPUTS, PreparedTokenPull, PullReceiver, ReservationTarget, TokenAllowance,
        TokenAllowanceFailure, TokenAllowanceStatus, TokenOutput, TokenOutputServiceError,
        TokenOutputWithPrevOut, TokensConfig, bech32m_decode_token_id,
        operations::transactions::unix_micros, select_token_outputs_from,
    },
};

use super::{
    error::{classify_allowance_error, operator_status},
    model::PULL_VALIDITY_SECONDS,
    service::{TokenAllowanceService, now_millis},
};

const MAX_PULL_RECEIVERS: usize = MAX_TOKEN_TX_OUTPUTS - 1;

impl TokenAllowanceService {
    pub async fn prepare_token_pull(
        &self,
        payer: PublicKey,
        token_identifier: &str,
        receivers: Vec<PullReceiver>,
    ) -> Result<PreparedTokenPull, ServiceError> {
        let spender = self.spark_signer.get_identity_public_key().await?;
        if payer == spender {
            return Err(ServiceError::InvalidInput(
                "the payer must differ from this wallet".to_string(),
            ));
        }
        let total = validate_pull_receivers(&receivers, payer)?;
        let raw_token_id = bech32m_decode_token_id(token_identifier, Some(self.network))?;
        let info = self
            .find_pull_allowance(payer, spender, &raw_token_id)
            .await?;
        let now_secs = now_millis()? / 1000;
        let allowance = TokenAllowance::from_info(&info, self.network, now_secs)?;
        check_pull_against_allowance(&allowance, &receivers, now_secs)?;
        let (free, pending) = self.fetch_payer_outputs(payer, &raw_token_id).await?;
        let selected = select_token_outputs_from(
            token_identifier,
            pull_candidates(free, pending, total),
            ReservationTarget::MinTotalValue(total),
            None,
        )
        .map_err(|e| match e {
            TokenOutputServiceError::InsufficientFunds { .. } => {
                TokenAllowanceFailure::PayerInsufficientFunds
                    .into_error("the payer's balance can't cover the pull")
            }
            other => other.into(),
        })?;
        build_pull(PullBuildParams {
            allowance_id: allowance.id,
            allowance_expiry_time: allowance.expiry_time,
            payer,
            token_identifier,
            receivers,
            selected,
            network: self.network,
            operator_identity_public_keys: self
                .operator_pool
                .get_all_operators()
                .map(|o| o.identity_public_key.serialize().to_vec())
                .collect(),
            tokens_config: &self.tokens_config,
            now: SystemTime::now(),
        })
    }

    pub async fn sign_and_broadcast_token_pull(
        &self,
        prepared: &PreparedTokenPull,
    ) -> Result<TokenTransaction, ServiceError> {
        let signature = self
            .sign(
                TokenTransactionKind::AllowanceSpend,
                prepared.spend_digest()?,
            )
            .await?;
        self.broadcast_token_pull(prepared, &signature).await
    }

    pub async fn broadcast_token_pull(
        &self,
        prepared: &PreparedTokenPull,
        signature: &schnorr::Signature,
    ) -> Result<TokenTransaction, ServiceError> {
        let spender = self.spark_signer.get_identity_public_key().await?;
        let request = build_pull_broadcast_request(prepared, spender, signature)?;
        let window_passed = now_millis()? / 1000 >= prepared.window_end();
        let response = self
            .operator_pool
            .get_coordinator()
            .client
            .broadcast_transaction(request)
            .await
            .map_err(|e| classify_pull_error(e.into(), window_passed))?;
        if response.commit_status() != CommitStatus::CommitFinalized {
            warn!(
                "Pull {} broadcast returned {:?}; committed operators: {:?}, \
                 uncommitted operators: {:?}",
                hex::encode(&prepared.partial_token_transaction_hash),
                response.commit_status(),
                response
                    .commit_progress
                    .as_ref()
                    .map(|p| &p.committed_operator_public_keys),
                response
                    .commit_progress
                    .as_ref()
                    .map(|p| &p.uncommitted_operator_public_keys),
            );
        }
        pull_transaction(response, prepared.created_timestamp, self.network)
    }

    async fn find_pull_allowance(
        &self,
        payer: PublicKey,
        spender: PublicKey,
        raw_token_id: &[u8],
    ) -> Result<TokenAllowanceInfo, ServiceError> {
        let active = self.query_pair(payer, spender, raw_token_id, false).await?;
        let records = if active.is_empty() {
            self.query_pair(payer, spender, raw_token_id, true).await?
        } else {
            active
        };
        choose_pull_allowance(records)
    }

    async fn fetch_payer_outputs(
        &self,
        payer: PublicKey,
        raw_token_id: &[u8],
    ) -> Result<(Vec<TokenOutputWithPrevOut>, Vec<TokenOutputWithPrevOut>), ServiceError> {
        let outputs = self
            .operator_pool
            .get_coordinator()
            .client
            .query_all_token_outputs(QueryAllTokenOutputsRequest {
                owner_public_keys: vec![payer.serialize().to_vec()],
                token_identifiers: vec![raw_token_id.to_vec()],
                network: self.network.to_proto_network() as i32,
                ..Default::default()
            })
            .await?;
        let (pending, free): (Vec<_>, Vec<_>) = outputs.into_iter().partition(is_pending_outbound);
        let network = self.network;
        let convert = |outputs: Vec<OutputWithPreviousTransactionData>| {
            outputs
                .into_iter()
                .map(|o| (o, network).try_into())
                .collect::<Result<Vec<TokenOutputWithPrevOut>, ServiceError>>()
        };
        Ok((convert(free)?, convert(pending)?))
    }

    pub(super) async fn query_pair(
        &self,
        owner: PublicKey,
        spender: PublicKey,
        raw_token_id: &[u8],
        include_inactive: bool,
    ) -> Result<Vec<TokenAllowanceInfo>, ServiceError> {
        self.query(
            QueryTokenAllowancesRequest {
                owner_public_key: Some(owner.serialize().to_vec()),
                spender_public_key: Some(spender.serialize().to_vec()),
                token_identifier: Some(raw_token_id.to_vec()),
                include_inactive,
                ..Default::default()
            },
            usize::MAX,
        )
        .await
    }
}

fn choose_pull_allowance(
    records: Vec<TokenAllowanceInfo>,
) -> Result<TokenAllowanceInfo, ServiceError> {
    let latest_active_first = records.into_iter().max_by_key(|info| {
        (
            info.status() == ProtoAllowanceStatus::Active,
            info.allowance_payload
                .as_ref()
                .map_or(0, |p| p.owner_provided_timestamp),
        )
    });
    let Some(info) = latest_active_first else {
        return Err(TokenAllowanceFailure::NotFound
            .into_error("no token allowance from this payer for this token"));
    };
    match info.status() {
        ProtoAllowanceStatus::Active | ProtoAllowanceStatus::Exhausted => Ok(info),
        ProtoAllowanceStatus::Revoked => {
            Err(TokenAllowanceFailure::Revoked.into_error("the token allowance was revoked"))
        }
        ProtoAllowanceStatus::Expired => {
            Err(TokenAllowanceFailure::Expired.into_error("the token allowance has expired"))
        }
        ProtoAllowanceStatus::Unspecified => Err(TokenAllowanceFailure::NotFound
            .into_error("no token allowance from this payer for this token")),
    }
}

fn is_pending_outbound(output: &OutputWithPreviousTransactionData) -> bool {
    output.output.as_ref().and_then(|o| o.status) == Some(TokenOutputStatus::PendingOutbound as i32)
}

fn pull_transaction(
    response: BroadcastTransactionResponse,
    created_timestamp: SystemTime,
    network: Network,
) -> Result<TokenTransaction, ServiceError> {
    let status = match response.commit_status() {
        CommitStatus::CommitFinalized => TokenTransactionStatus::Finalized,
        CommitStatus::CommitProcessing | CommitStatus::CommitUnspecified => {
            TokenTransactionStatus::Unknown
        }
    };
    let final_tx = response.final_token_transaction.ok_or_else(|| {
        ServiceError::Generic("broadcast response missing final_token_transaction".to_string())
    })?;
    let hash = hex::encode(
        spark_primitives::hash_final_token_transaction(final_tx.encode_to_vec())
            .map_err(|e| ServiceError::Generic(e.to_string()))?,
    );
    let inputs = final_tx
        .token_inputs
        .ok_or_else(|| ServiceError::Generic("final pull transaction missing inputs".to_string()))?
        .try_into()?;
    let outputs = final_tx
        .final_token_outputs
        .into_iter()
        .map(|output| (output, network).try_into())
        .collect::<Result<Vec<TokenOutput>, ServiceError>>()?;
    Ok(TokenTransaction {
        hash,
        inputs,
        outputs,
        status,
        created_timestamp,
        fulfilled_invoices: Vec::new(),
    })
}

const TRANSACTION_PREEMPTED: &str = "TRANSACTION_PREEMPTED";
const OUT_OF_RANGE: &str = "OUT_OF_RANGE";

fn pull_never_accepted(status: &Status, window_passed: bool) -> bool {
    let reason = status.get_details_error_info().map(|info| info.reason);
    match (status.code(), reason.as_deref()) {
        (Code::Aborted, Some(TRANSACTION_PREEMPTED)) => true,
        (Code::InvalidArgument, Some(OUT_OF_RANGE)) => window_passed,
        _ => false,
    }
}

fn classify_pull_error(error: ServiceError, window_passed: bool) -> ServiceError {
    let stale = operator_status(&error)
        .filter(|status| pull_never_accepted(status, window_passed))
        .map(|status| status.message().to_string());
    match stale {
        Some(message) => TokenAllowanceFailure::PreparedPullStale.into_error(message),
        None => classify_allowance_error(error),
    }
}

pub(crate) struct PullBuildParams<'a> {
    pub allowance_id: String,
    pub allowance_expiry_time: u64,
    pub payer: PublicKey,
    pub token_identifier: &'a str,
    pub receivers: Vec<PullReceiver>,
    pub selected: Vec<TokenOutputWithPrevOut>,
    pub network: Network,
    pub operator_identity_public_keys: Vec<Vec<u8>>,
    pub tokens_config: &'a TokensConfig,
    pub now: SystemTime,
}

pub(crate) fn validate_pull_receivers(
    receivers: &[PullReceiver],
    payer: PublicKey,
) -> Result<u128, ServiceError> {
    if receivers.is_empty() {
        return Err(ServiceError::InvalidInput(
            "a pull needs at least one receiver".to_string(),
        ));
    }
    if receivers.len() > MAX_PULL_RECEIVERS {
        return Err(ServiceError::InvalidInput(format!(
            "a pull is limited to {MAX_PULL_RECEIVERS} receivers"
        )));
    }
    let mut seen = HashSet::new();
    let mut total: u128 = 0;
    for receiver in receivers {
        if receiver.amount == 0 {
            return Err(ServiceError::InvalidInput(
                "receiver amounts must be positive".to_string(),
            ));
        }
        if receiver.receiver_public_key == payer {
            return Err(ServiceError::InvalidInput(
                "the payer can't be a receiver".to_string(),
            ));
        }
        if !seen.insert(receiver.receiver_public_key) {
            return Err(ServiceError::InvalidInput(
                "receivers must be distinct".to_string(),
            ));
        }
        total = total
            .checked_add(receiver.amount)
            .ok_or_else(|| ServiceError::InvalidInput("pull total overflows".to_string()))?;
    }
    Ok(total)
}

pub(crate) fn check_pull_against_allowance(
    allowance: &TokenAllowance,
    receivers: &[PullReceiver],
    now_secs: u64,
) -> Result<(), ServiceError> {
    match allowance.status {
        TokenAllowanceStatus::Revoked => {
            return Err(
                TokenAllowanceFailure::Revoked.into_error("the token allowance was revoked")
            );
        }
        TokenAllowanceStatus::Expired => {
            return Err(
                TokenAllowanceFailure::Expired.into_error("the token allowance has expired")
            );
        }
        TokenAllowanceStatus::Active | TokenAllowanceStatus::Exhausted => {}
    }
    if allowance.expiry_time <= now_secs {
        return Err(TokenAllowanceFailure::Expired.into_error("the token allowance has expired"));
    }
    let total: u128 = receivers.iter().map(|r| r.amount).sum();
    if let Some(cap) = allowance.max_per_payment
        && total > cap
    {
        return Err(TokenAllowanceFailure::OverPerPaymentLimit
            .into_error("the amount exceeds the per-payment limit"));
    }
    if !allowance.allowed_recipients.is_empty()
        && receivers.iter().any(|r| {
            !allowance
                .allowed_recipients
                .contains(&r.receiver_public_key)
        })
    {
        return Err(TokenAllowanceFailure::RecipientNotAllowed
            .into_error("a receiver is not on the allowance's recipient list"));
    }
    Ok(())
}

pub(crate) fn pull_candidates(
    free: Vec<TokenOutputWithPrevOut>,
    pending: Vec<TokenOutputWithPrevOut>,
    total: u128,
) -> Vec<TokenOutputWithPrevOut> {
    let free_total = free
        .iter()
        .fold(0u128, |sum, o| sum.saturating_add(o.output.token_amount));
    if free_total >= total {
        free
    } else {
        free.into_iter().chain(pending).collect()
    }
}

pub(crate) fn build_pull(params: PullBuildParams<'_>) -> Result<PreparedTokenPull, ServiceError> {
    let PullBuildParams {
        allowance_id,
        allowance_expiry_time,
        payer,
        token_identifier,
        receivers,
        mut selected,
        network,
        operator_identity_public_keys,
        tokens_config,
        now,
    } = params;
    if selected.len() > tokens_config.max_tx_inputs {
        return Err(ServiceError::NeededTooManyOutputs);
    }
    selected.sort_by_key(|o| o.prev_tx_vout);
    let raw_token_id = bech32m_decode_token_id(token_identifier, Some(network))?;
    let selected_outputs = selected
        .iter()
        .map(|o| {
            Ok(spark_primitives::SelectedTokenOutput {
                previous_transaction_hash: hex::decode(&o.prev_tx_hash)
                    .map_err(|_| ServiceError::Generic("invalid prev tx hash".to_string()))?,
                previous_transaction_vout: o.prev_tx_vout,
                owner_public_key: o.output.owner_public_key.serialize().to_vec(),
                token_identifier: raw_token_id.clone(),
                token_amount: o.output.token_amount.to_be_bytes().to_vec(),
            })
        })
        .collect::<Result<Vec<_>, ServiceError>>()?;
    let receiver_outputs = receivers
        .iter()
        .map(|r| {
            Ok(spark_primitives::ReceiverTokenOutput {
                receiver_spark_address: SparkAddress::new(r.receiver_public_key, network, None)
                    .to_address_string()
                    .map_err(|e| ServiceError::Generic(e.to_string()))?,
                token_identifier: Some(raw_token_id.clone()),
                token_amount: Some(r.amount.to_be_bytes().to_vec()),
            })
        })
        .collect::<Result<Vec<_>, ServiceError>>()?;
    let proto_network = u32::try_from(network.to_proto_network() as i32)
        .map_err(|_| ServiceError::Generic("network proto value is negative".to_string()))?;
    let built = spark_primitives::construct_partial_transfer_transaction(
        spark_primitives::TransferBuildRequest {
            identity_public_key: payer.serialize().to_vec(),
            selected_outputs,
            receiver_outputs,
            operator_identity_public_keys,
            network: proto_network,
            validity_duration_seconds: PULL_VALIDITY_SECONDS,
            client_created_timestamp_unix_micros: unix_micros(now)?,
            withdraw_bond_sats: tokens_config.expected_withdraw_bond_sats,
            withdraw_relative_block_locktime: tokens_config
                .expected_withdraw_relative_block_locktime,
            execute_before_unix_micros: None,
        },
    )
    .map_err(|e| ServiceError::Generic(e.to_string()))?;
    Ok(PreparedTokenPull {
        allowance_id,
        payer_public_key: payer,
        token_identifier: token_identifier.to_string(),
        receivers,
        partial_token_transaction_bytes: built.partial_token_transaction_bytes,
        partial_token_transaction_hash: built.partial_token_transaction_hash,
        created_timestamp: now,
        allowance_expiry_time,
    })
}

pub(crate) fn build_pull_broadcast_request(
    prepared: &PreparedTokenPull,
    spender: PublicKey,
    signature: &schnorr::Signature,
) -> Result<BroadcastTransactionRequest, ServiceError> {
    let partial =
        PartialTokenTransaction::decode(prepared.partial_token_transaction_bytes.as_slice())
            .map_err(|e| ServiceError::Generic(format!("invalid pull transaction: {e}")))?;
    let input_count = match &partial.token_inputs {
        Some(TokenInputs::TransferInput(transfer)) => transfer.outputs_to_spend.len(),
        _ => {
            return Err(ServiceError::Generic(
                "a pull transaction must spend transfer inputs".to_string(),
            ));
        }
    };
    let allowance_id = Uuid::parse_str(&prepared.allowance_id)
        .map_err(|e| ServiceError::InvalidInput(format!("invalid allowance id: {e}")))?
        .as_bytes()
        .to_vec();
    let spender_bytes = spender.serialize().to_vec();
    let signature_bytes = signature.serialize().to_vec();
    let signatures = (0..input_count)
        .map(|index| {
            Ok(SignatureWithIndex {
                signature: None,
                input_index: u32::try_from(index)
                    .map_err(|_| ServiceError::Generic("too many inputs".to_string()))?,
                authority_signatures: Some(AuthoritySignatures::AllowanceSignature(
                    AllowanceSignature {
                        allowance_id: allowance_id.clone(),
                        spender_signature: Some(KeyedSignature {
                            public_key: spender_bytes.clone(),
                            signature: signature_bytes.clone(),
                        }),
                    },
                )),
            })
        })
        .collect::<Result<Vec<_>, ServiceError>>()?;
    Ok(BroadcastTransactionRequest {
        identity_public_key: spender_bytes,
        partial_token_transaction: Some(partial),
        token_transaction_owner_signatures: signatures,
    })
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use macros::test_all;
    use platform_utils::time::{SystemTime, UNIX_EPOCH};
    use prost::Message as _;
    use tonic::{Code, Status};
    use tonic_types::{ErrorDetails, StatusExt};
    use uuid::Uuid;

    use super::{
        PULL_VALIDITY_SECONDS, PreparedTokenPull, PullBuildParams, PullReceiver, build_pull,
        check_pull_against_allowance, choose_pull_allowance, classify_pull_error, pull_candidates,
        pull_transaction, validate_pull_receivers,
    };
    use crate::{
        Network,
        operator::rpc::{
            OperatorRpcError,
            spark_token::{
                BroadcastTransactionResponse, CommitStatus, FinalTokenOutput,
                FinalTokenTransaction, PartialTokenOutput, PartialTokenTransaction,
                TokenAllowanceInfo, TokenAllowancePayload, TokenAllowanceStatus as Proto,
                TokenOutputToSpend, TokenTransferInput, final_token_transaction,
            },
        },
        services::{ServiceError, TokenInputs, TokenTransaction, TokenTransactionStatus},
        token::{
            DEFAULT_MAX_TOKEN_TX_INPUTS, MAX_TOKEN_TX_OUTPUTS, TokenAllowance,
            TokenAllowanceFailure, TokenAllowanceStatus, TokenOutput, TokenOutputWithPrevOut,
            TokensConfig, bech32m_encode_token_id,
        },
    };

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn key(fill: u8) -> PublicKey {
        PublicKey::from_secret_key(
            &Secp256k1::new(),
            &SecretKey::from_slice(&[fill; 32]).unwrap(),
        )
    }

    fn token_id() -> String {
        bech32m_encode_token_id(&[7; 32], Network::Regtest).unwrap()
    }

    fn output(owner: PublicKey, amount: u128, vout: u32) -> TokenOutputWithPrevOut {
        TokenOutputWithPrevOut {
            output: TokenOutput {
                owner_public_key: owner,
                revocation_commitment: String::new(),
                withdraw_bond_sats: 10_000,
                withdraw_relative_block_locktime: 1_000,
                token_public_key: None,
                token_identifier: token_id(),
                token_amount: amount,
            },
            prev_tx_hash: "ab".repeat(32),
            prev_tx_vout: vout,
        }
    }

    #[test_all]
    fn a_pull_prefers_outputs_that_are_not_being_spent() {
        let free = vec![output(key(1), 60, 0), output(key(1), 50, 1)];
        let pending = vec![output(key(1), 10, 2)];
        assert_eq!(pull_candidates(free.clone(), pending.clone(), 100), free);
        assert_eq!(
            pull_candidates(free.clone(), pending.clone(), 120),
            [free, pending].concat()
        );
    }

    fn allowance() -> TokenAllowance {
        TokenAllowance {
            id: Uuid::now_v7().to_string(),
            owner_public_key: key(1),
            spender_public_key: key(2),
            token_identifier: token_id(),
            max_per_payment: Some(500),
            max_total: Some(1_000),
            spent_amount: 0,
            allowed_recipients: vec![],
            expiry_time: 2_000_000_000,
            created_at: 1_700_000_000,
            revoked_at: None,
            status: TokenAllowanceStatus::Active,
        }
    }

    fn failure(error: ServiceError) -> Option<TokenAllowanceFailure> {
        match error {
            ServiceError::TokenAllowance { failure, .. } => Some(failure),
            _ => None,
        }
    }

    #[test_all]
    fn builds_pull_with_change_to_payer() {
        let payer = key(1);
        let receiver = key(3);
        let prepared = build_pull(PullBuildParams {
            allowance_id: Uuid::now_v7().to_string(),
            allowance_expiry_time: 2_000_000_000,
            payer,
            token_identifier: &token_id(),
            receivers: vec![PullReceiver {
                receiver_public_key: receiver,
                amount: 100,
            }],
            selected: vec![output(payer, 250, 0)],
            network: Network::Regtest,
            operator_identity_public_keys: vec![
                key(4).serialize().to_vec(),
                key(5).serialize().to_vec(),
            ],
            tokens_config: &TokensConfig {
                expected_withdraw_bond_sats: 10_000,
                expected_withdraw_relative_block_locktime: 1_000,
                transaction_validity_duration_seconds: 180,
                max_tx_inputs: DEFAULT_MAX_TOKEN_TX_INPUTS,
            },
            now: SystemTime::now(),
        })
        .unwrap();

        let partial =
            PartialTokenTransaction::decode(prepared.partial_token_transaction_bytes.as_slice())
                .unwrap();
        let owners: Vec<Vec<u8>> = partial
            .partial_token_outputs
            .iter()
            .map(|o| o.owner_public_key.clone())
            .collect();
        assert_eq!(
            owners,
            vec![receiver.serialize().to_vec(), payer.serialize().to_vec()]
        );
        assert_eq!(
            partial
                .token_transaction_metadata
                .unwrap()
                .validity_duration_seconds,
            PULL_VALIDITY_SECONDS
        );
        assert_eq!(prepared.total(), 100);
        assert_eq!(prepared.spend_digest().unwrap().len(), 32);
    }

    #[test_all]
    fn a_pull_expires_with_its_window_or_its_allowance() {
        let pull = |allowance_expiry_time| PreparedTokenPull {
            allowance_id: Uuid::now_v7().to_string(),
            payer_public_key: key(1),
            token_identifier: token_id(),
            receivers: vec![],
            partial_token_transaction_bytes: vec![],
            partial_token_transaction_hash: vec![],
            created_timestamp: UNIX_EPOCH + Duration::from_secs(1_000),
            allowance_expiry_time,
        };
        assert_eq!(pull(2_000).expiry_time(), 1_000 + PULL_VALIDITY_SECONDS);
        assert_eq!(pull(1_100).expiry_time(), 1_100);
    }

    #[test_all]
    fn rejects_bad_receivers() {
        let payer = key(1);
        let receiver = |fill: u8, amount: u128| PullReceiver {
            receiver_public_key: key(fill),
            amount,
        };
        assert!(validate_pull_receivers(&[], payer).is_err());
        assert!(validate_pull_receivers(&[receiver(3, 0)], payer).is_err());
        assert!(validate_pull_receivers(&[receiver(1, 10)], payer).is_err());
        assert!(validate_pull_receivers(&[receiver(3, 10), receiver(3, 5)], payer).is_err());
        assert!(validate_pull_receivers(&[receiver(3, 10), receiver(4, 5)], payer).is_ok());
    }

    #[test_all]
    fn caps_receivers_to_leave_room_for_change() {
        let payer = key(1);
        let receivers = |count: u32| {
            (0..count)
                .map(|index| {
                    let mut secret = [7; 32];
                    secret[28..].copy_from_slice(&index.to_be_bytes());
                    PullReceiver {
                        receiver_public_key: PublicKey::from_secret_key(
                            &Secp256k1::new(),
                            &SecretKey::from_slice(&secret).unwrap(),
                        ),
                        amount: 1,
                    }
                })
                .collect::<Vec<_>>()
        };
        let cap = u32::try_from(MAX_TOKEN_TX_OUTPUTS - 1).unwrap();
        assert!(validate_pull_receivers(&receivers(cap), payer).is_ok());
        assert!(matches!(
            validate_pull_receivers(&receivers(cap + 1), payer),
            Err(ServiceError::InvalidInput(_))
        ));
    }

    #[test_all]
    fn checks_pull_against_allowance() {
        let to = |fill: u8, amount: u128| {
            vec![PullReceiver {
                receiver_public_key: key(fill),
                amount,
            }]
        };
        assert!(check_pull_against_allowance(&allowance(), &to(3, 500), 1_800_000_000).is_ok());
        assert_eq!(
            failure(
                check_pull_against_allowance(&allowance(), &to(3, 501), 1_800_000_000).unwrap_err()
            ),
            Some(TokenAllowanceFailure::OverPerPaymentLimit)
        );
        let listed = TokenAllowance {
            allowed_recipients: vec![key(4)],
            ..allowance()
        };
        assert_eq!(
            failure(check_pull_against_allowance(&listed, &to(3, 10), 1_800_000_000).unwrap_err()),
            Some(TokenAllowanceFailure::RecipientNotAllowed)
        );
        assert_eq!(
            failure(
                check_pull_against_allowance(&allowance(), &to(3, 10), 2_000_000_000).unwrap_err()
            ),
            Some(TokenAllowanceFailure::Expired)
        );
        let revoked = TokenAllowance {
            status: TokenAllowanceStatus::Revoked,
            ..allowance()
        };
        assert_eq!(
            failure(check_pull_against_allowance(&revoked, &to(3, 10), 1_800_000_000).unwrap_err()),
            Some(TokenAllowanceFailure::Revoked)
        );
    }

    #[test_all]
    fn broadcast_request_carries_allowance_signatures() {
        use bitcoin::secp256k1::{Keypair, Message};

        use super::build_pull_broadcast_request;
        use crate::operator::rpc::spark_token::signature_with_index::AuthoritySignatures;

        let payer = key(1);
        let secp = Secp256k1::new();
        let spender_secret = SecretKey::from_slice(&[9; 32]).unwrap();
        let spender = PublicKey::from_secret_key(&secp, &spender_secret);
        let allowance_id = Uuid::now_v7();
        let prepared = build_pull(PullBuildParams {
            allowance_id: allowance_id.to_string(),
            allowance_expiry_time: 2_000_000_000,
            payer,
            token_identifier: &token_id(),
            receivers: vec![PullReceiver {
                receiver_public_key: spender,
                amount: 300,
            }],
            selected: vec![output(payer, 200, 0), output(payer, 200, 1)],
            network: Network::Regtest,
            operator_identity_public_keys: vec![key(4).serialize().to_vec()],
            tokens_config: &TokensConfig {
                expected_withdraw_bond_sats: 10_000,
                expected_withdraw_relative_block_locktime: 1_000,
                transaction_validity_duration_seconds: 180,
                max_tx_inputs: DEFAULT_MAX_TOKEN_TX_INPUTS,
            },
            now: SystemTime::now(),
        })
        .unwrap();
        let signature = secp.sign_schnorr_no_aux_rand(
            &Message::from_digest(prepared.spend_digest().unwrap()),
            &Keypair::from_secret_key(&secp, &spender_secret),
        );

        let request = build_pull_broadcast_request(&prepared, spender, &signature).unwrap();

        assert_eq!(request.identity_public_key, spender.serialize().to_vec());
        assert_eq!(request.token_transaction_owner_signatures.len(), 2);
        for (index, entry) in request
            .token_transaction_owner_signatures
            .iter()
            .enumerate()
        {
            assert_eq!(entry.input_index as usize, index);
            let Some(AuthoritySignatures::AllowanceSignature(allowance_signature)) =
                &entry.authority_signatures
            else {
                panic!("expected an allowance signature");
            };
            assert_eq!(
                allowance_signature.allowance_id,
                allowance_id.as_bytes().to_vec()
            );
            let keyed = allowance_signature.spender_signature.as_ref().unwrap();
            assert_eq!(keyed.public_key, spender.serialize().to_vec());
            assert_eq!(keyed.signature, signature.serialize().to_vec());
        }
    }

    fn rpc_error(code: Code, message: &str) -> ServiceError {
        ServiceError::ServiceConnectionError(Box::new(OperatorRpcError::Connection(Box::new(
            Status::new(code, message),
        ))))
    }

    fn error_with_reason(code: Code, reason: &str) -> ServiceError {
        let details = ErrorDetails::with_error_info(reason, "spark", HashMap::new());
        ServiceError::ServiceConnectionError(Box::new(OperatorRpcError::Connection(Box::new(
            Status::with_error_details(code, "operator rejected", details),
        ))))
    }

    #[test_all]
    fn a_pull_the_operators_never_accepted_is_stale() {
        let stale = [
            (Code::Aborted, "TRANSACTION_PREEMPTED", false),
            (Code::InvalidArgument, "OUT_OF_RANGE", true),
        ];
        for (code, reason, window_passed) in stale {
            assert_eq!(
                failure(classify_pull_error(
                    error_with_reason(code, reason),
                    window_passed
                )),
                Some(TokenAllowanceFailure::PreparedPullStale)
            );
        }
    }

    #[test_all]
    fn other_pull_errors_are_not_stale() {
        let others = [
            (
                error_with_reason(Code::InvalidArgument, "OUT_OF_RANGE"),
                false,
            ),
            (error_with_reason(Code::Aborted, "LOCK_CONFLICT"), true),
            (
                error_with_reason(Code::AlreadyExists, "EXPIRED_TRANSACTION"),
                true,
            ),
            (
                error_with_reason(Code::AlreadyExists, "DUPLICATE_OPERATION"),
                true,
            ),
            (rpc_error(Code::Aborted, "inputs cannot be spent"), true),
        ];
        for (error, window_passed) in others {
            assert!(matches!(
                classify_pull_error(error, window_passed),
                ServiceError::ServiceConnectionError(_)
            ));
        }
    }

    fn record(tag: u8, status: Proto, owner_provided_timestamp: u64) -> TokenAllowanceInfo {
        TokenAllowanceInfo {
            allowance_payload: Some(TokenAllowancePayload {
                allowance_id: vec![tag; 16],
                owner_provided_timestamp,
                ..Default::default()
            }),
            status: status as i32,
            ..Default::default()
        }
    }

    fn chosen(records: Vec<TokenAllowanceInfo>) -> u8 {
        choose_pull_allowance(records)
            .unwrap()
            .allowance_payload
            .unwrap()
            .allowance_id[0]
    }

    fn refusal(records: Vec<TokenAllowanceInfo>) -> Option<TokenAllowanceFailure> {
        match choose_pull_allowance(records) {
            Err(ServiceError::TokenAllowance { failure, .. }) => Some(failure),
            _ => None,
        }
    }

    #[test_all]
    fn an_active_grant_wins() {
        assert_eq!(
            chosen(vec![
                record(1, Proto::Exhausted, 3),
                record(2, Proto::Active, 1),
                record(3, Proto::Revoked, 5),
            ]),
            2
        );
    }

    #[test_all]
    fn no_records_is_not_found() {
        assert_eq!(refusal(vec![]), Some(TokenAllowanceFailure::NotFound));
    }

    #[test_all]
    fn the_latest_revoked_grant_refuses() {
        assert_eq!(
            refusal(vec![
                record(1, Proto::Expired, 1),
                record(2, Proto::Revoked, 2)
            ]),
            Some(TokenAllowanceFailure::Revoked)
        );
    }

    #[test_all]
    fn the_latest_expired_grant_refuses() {
        assert_eq!(
            refusal(vec![
                record(1, Proto::Revoked, 1),
                record(2, Proto::Expired, 2)
            ]),
            Some(TokenAllowanceFailure::Expired)
        );
    }

    #[test_all]
    fn the_latest_exhausted_grant_is_used() {
        assert_eq!(
            chosen(vec![
                record(1, Proto::Revoked, 1),
                record(2, Proto::Exhausted, 2)
            ]),
            2
        );
    }

    #[test_all]
    fn a_newer_revoked_grant_beats_an_older_exhausted_one() {
        assert_eq!(
            refusal(vec![
                record(1, Proto::Exhausted, 1),
                record(2, Proto::Revoked, 2)
            ]),
            Some(TokenAllowanceFailure::Revoked)
        );
    }

    fn broadcast_response(status: CommitStatus) -> BroadcastTransactionResponse {
        let receiver = PublicKey::from_secret_key(
            &Secp256k1::new(),
            &SecretKey::from_slice(&[1; 32]).unwrap(),
        );
        BroadcastTransactionResponse {
            final_token_transaction: Some(FinalTokenTransaction {
                version: 3,
                token_inputs: Some(final_token_transaction::TokenInputs::TransferInput(
                    TokenTransferInput {
                        outputs_to_spend: vec![TokenOutputToSpend {
                            prev_token_transaction_hash: vec![4; 32],
                            prev_token_transaction_vout: 2,
                        }],
                    },
                )),
                final_token_outputs: vec![FinalTokenOutput {
                    partial_token_output: Some(PartialTokenOutput {
                        owner_public_key: receiver.serialize().to_vec(),
                        withdraw_bond_sats: 10_000,
                        withdraw_relative_block_locktime: 1_000,
                        token_identifier: vec![7; 32],
                        token_amount: 42u128.to_be_bytes().to_vec(),
                    }),
                    revocation_commitment: vec![2; 33],
                }],
                ..Default::default()
            }),
            commit_status: status as i32,
            ..Default::default()
        }
    }

    fn mapped(status: CommitStatus) -> (Result<TokenTransaction, ServiceError>, String) {
        let response = broadcast_response(status);
        let final_hash = hex::encode(
            spark_primitives::hash_final_token_transaction(
                response
                    .final_token_transaction
                    .as_ref()
                    .unwrap()
                    .encode_to_vec(),
            )
            .unwrap(),
        );
        (
            pull_transaction(response, SystemTime::UNIX_EPOCH, Network::Regtest),
            final_hash,
        )
    }

    fn assert_carries_final_transaction(transaction: &TokenTransaction, final_hash: &str) {
        assert_eq!(transaction.hash, final_hash);
        let TokenInputs::Transfer(input) = &transaction.inputs else {
            panic!("expected transfer inputs");
        };
        assert_eq!(input.outputs_to_spend.len(), 1);
        assert_eq!(
            input.outputs_to_spend[0].prev_token_tx_hash,
            "04".repeat(32)
        );
        assert_eq!(input.outputs_to_spend[0].prev_token_tx_vout, 2);
        assert_eq!(transaction.outputs.len(), 1);
        assert_eq!(transaction.outputs[0].token_amount, 42);
        assert_eq!(transaction.created_timestamp, SystemTime::UNIX_EPOCH);
    }

    #[test_all]
    fn a_finalized_broadcast_returns_the_final_pull() {
        let (transaction, final_hash) = mapped(CommitStatus::CommitFinalized);
        let transaction = transaction.unwrap();
        assert!(matches!(
            transaction.status,
            TokenTransactionStatus::Finalized
        ));
        assert_carries_final_transaction(&transaction, &final_hash);
    }

    #[test_all]
    fn a_broadcast_that_is_not_final_returns_the_pending_pull() {
        for status in [
            CommitStatus::CommitProcessing,
            CommitStatus::CommitUnspecified,
        ] {
            let (transaction, final_hash) = mapped(status);
            let transaction = transaction.unwrap();
            assert!(matches!(
                transaction.status,
                TokenTransactionStatus::Unknown
            ));
            assert_carries_final_transaction(&transaction, &final_hash);
        }
    }
}
