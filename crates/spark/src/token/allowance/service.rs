use std::sync::Arc;

use bitcoin::secp256k1::{PublicKey, schnorr};
use platform_utils::time::{SystemTime, UNIX_EPOCH};
use prost::Message as _;
use tonic::Code;
use tracing::warn;
use uuid::Uuid;

use crate::{
    Network,
    operator::{
        OperatorPool,
        rpc::{
            OperatorRpcError, QueryAllTokenOutputsRequest,
            spark_token::{
                BroadcastTransactionResponse, CommitStatus, CreateTokenAllowanceRequest,
                OutputWithPreviousTransactionData, QueryTokenAllowancesRequest,
                QueryTokenAllowancesResponse, RevokeTokenAllowanceRequest, TokenAllowanceInfo,
                TokenAllowanceStatus as ProtoAllowanceStatus, TokenOutputStatus,
            },
        },
    },
    services::{ServiceError, TokenTransaction, TokenTransactionStatus},
    signer::{PrepareTokenTransactionRequest, SparkSigner, TokenTransactionKind},
    token::{
        ReservationTarget, TokenAllowanceFailure, TokenOutput, TokenOutputServiceError,
        TokenOutputWithPrevOut, TokensConfig, bech32m_decode_token_id, select_token_outputs_from,
    },
};

use super::{
    error::{classify_allowance_error, classify_pull_error},
    hash::{hash_create_statement, hash_revoke_statement},
    model::{
        NewTokenAllowance, TokenAllowance, TokenAllowanceQuery, TokenAllowanceRole, revoke_payload,
    },
    pull::{
        PreparedTokenPull, PullBuildParams, PullReceiver, build_pull, build_pull_broadcast_request,
        check_pull_against_allowance, pull_candidates, validate_pull_receivers,
    },
    verify::{verify_allowance_record, verify_created_record, verify_queried_record},
};

const CREATE_ATTEMPTS: usize = 3;
const QUERY_PAGE_LIMIT: u8 = 100;
const MAX_QUERY_PAGES: usize = 20;

pub struct TokenAllowanceService {
    pub(super) spark_signer: Arc<dyn SparkSigner>,
    pub(super) operator_pool: Arc<OperatorPool>,
    pub(super) network: Network,
    pub(super) tokens_config: TokensConfig,
}

pub(crate) fn now_millis() -> Result<u64, ServiceError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ServiceError::Generic("system time is before UNIX_EPOCH".to_string()))?;
    u64::try_from(elapsed.as_millis())
        .map_err(|_| ServiceError::Generic("system time overflows u64".to_string()))
}

fn is_transient(error: &OperatorRpcError) -> bool {
    matches!(
        error,
        OperatorRpcError::Connection(status)
            if matches!(status.code(), Code::Unavailable | Code::DeadlineExceeded)
    )
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

impl TokenAllowanceService {
    pub fn new(
        spark_signer: Arc<dyn SparkSigner>,
        operator_pool: Arc<OperatorPool>,
        network: Network,
        tokens_config: TokensConfig,
    ) -> Self {
        Self {
            spark_signer,
            operator_pool,
            network,
            tokens_config,
        }
    }

    pub(super) async fn sign(
        &self,
        kind: TokenTransactionKind,
        digest: [u8; 32],
    ) -> Result<schnorr::Signature, ServiceError> {
        Ok(self
            .spark_signer
            .prepare_token_transaction(PrepareTokenTransactionRequest { kind, digest })
            .await?
            .signature)
    }

    pub async fn create_token_allowance(
        &self,
        request: NewTokenAllowance,
    ) -> Result<TokenAllowance, ServiceError> {
        let owner = self.spark_signer.get_identity_public_key().await?;
        let payload = request.to_payload(Uuid::now_v7(), owner, self.network, now_millis()?)?;
        let owner_signature = self
            .sign(
                TokenTransactionKind::AllowanceGrant,
                hash_create_statement(&payload)?,
            )
            .await?
            .serialize()
            .to_vec();
        let rpc_request = CreateTokenAllowanceRequest {
            allowance_payload: Some(payload.clone()),
            owner_signature: owner_signature.clone(),
        };
        let mut attempt = 1;
        let response = loop {
            match self
                .operator_pool
                .get_coordinator()
                .client
                .create_token_allowance(rpc_request.clone())
                .await
            {
                Ok(response) => break response,
                Err(e) if is_transient(&e) && attempt < CREATE_ATTEMPTS => attempt += 1,
                Err(e) => return Err(classify_allowance_error(e.into())),
            }
        };
        let info = match response.allowance {
            Some(info) => {
                verify_allowance_record(&info)?;
                verify_created_record(&info, &payload)?;
                info
            }
            None => TokenAllowanceInfo {
                allowance_payload: Some(payload),
                spent_amount: Vec::new(),
                status: crate::operator::rpc::spark_token::TokenAllowanceStatus::Active as i32,
                owner_signature,
                revoke_signature: Vec::new(),
                owner_provided_revoke_timestamp: 0,
                revoke_version: 0,
            },
        };
        TokenAllowance::from_info(&info, self.network, now_millis()? / 1000)
    }

    pub async fn revoke_token_allowance(&self, allowance_id: &str) -> Result<(), ServiceError> {
        let allowance_id = Uuid::parse_str(allowance_id)
            .map_err(|e| ServiceError::InvalidInput(format!("invalid allowance id: {e}")))?;
        let owner = self.spark_signer.get_identity_public_key().await?;
        let payload = revoke_payload(allowance_id, owner, now_millis()?);
        let owner_signature = self
            .sign(
                TokenTransactionKind::AllowanceRevoke,
                hash_revoke_statement(&payload)?,
            )
            .await?
            .serialize()
            .to_vec();
        self.operator_pool
            .get_coordinator()
            .client
            .revoke_token_allowance(RevokeTokenAllowanceRequest {
                revoke_allowance_payload: Some(payload),
                owner_signature,
            })
            .await
            .map_err(|e| classify_allowance_error(e.into()))?;
        Ok(())
    }

    pub async fn query_token_allowances(
        &self,
        query: TokenAllowanceQuery,
    ) -> Result<Vec<TokenAllowance>, ServiceError> {
        let own = self
            .spark_signer
            .get_identity_public_key()
            .await?
            .serialize()
            .to_vec();
        let counterparty = query
            .counterparty_public_key
            .map(|k| k.serialize().to_vec());
        let (owner_public_key, spender_public_key) = match query.role {
            TokenAllowanceRole::Owner => (Some(own), counterparty),
            TokenAllowanceRole::Spender => (counterparty, Some(own)),
        };
        let token_identifier = query
            .token_identifier
            .as_deref()
            .map(|t| bech32m_decode_token_id(t, Some(self.network)))
            .transpose()?;
        let records = self
            .query(
                QueryTokenAllowancesRequest {
                    owner_public_key,
                    spender_public_key,
                    token_identifier,
                    include_inactive: query.include_inactive,
                    offset: i64::from(query.offset),
                    ..Default::default()
                },
                usize::try_from(query.limit).unwrap_or(usize::MAX),
            )
            .await?;
        let now_secs = now_millis()? / 1000;
        records
            .iter()
            .map(|info| TokenAllowance::from_info(info, self.network, now_secs))
            .collect()
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

    async fn query(
        &self,
        request: QueryTokenAllowancesRequest,
        row_cap: usize,
    ) -> Result<Vec<TokenAllowanceInfo>, ServiceError> {
        let client = &self.operator_pool.get_coordinator().client;
        let records = read_pages(&request, row_cap, |page| async move {
            client
                .query_token_allowances(page)
                .await
                .map_err(|e| classify_allowance_error(e.into()))
        })
        .await?;
        for info in &records {
            verify_allowance_record(info)?;
            verify_queried_record(info, &request)?;
        }
        Ok(records)
    }

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
}

fn is_pending_outbound(output: &OutputWithPreviousTransactionData) -> bool {
    output.output.as_ref().and_then(|o| o.status) == Some(TokenOutputStatus::PendingOutbound as i32)
}

async fn read_pages<F, Fut>(
    request: &QueryTokenAllowancesRequest,
    row_cap: usize,
    mut send: F,
) -> Result<Vec<TokenAllowanceInfo>, ServiceError>
where
    F: FnMut(QueryTokenAllowancesRequest) -> Fut,
    Fut: Future<Output = Result<QueryTokenAllowancesResponse, ServiceError>>,
{
    let mut records = Vec::new();
    let mut offset = request.offset;
    for _ in 0..MAX_QUERY_PAGES {
        let wanted = row_cap.saturating_sub(records.len());
        if wanted == 0 {
            return Ok(records);
        }
        let limit = u8::try_from(wanted).map_or(QUERY_PAGE_LIMIT, |w| w.min(QUERY_PAGE_LIMIT));
        let page = send(QueryTokenAllowancesRequest {
            limit: i64::from(limit),
            offset,
            ..request.clone()
        })
        .await?;
        let last =
            page.offset < 0 || page.offset <= offset || page.allowances.len() < usize::from(limit);
        records.extend(page.allowances.into_iter().take(usize::from(limit)));
        if last {
            return Ok(records);
        }
        offset = page.offset;
    }
    if records.len() < row_cap {
        return Err(ServiceError::InvalidInput(
            "too many token allowances match; filter by token or counterparty".to_string(),
        ));
    }
    Ok(records)
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

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, future::ready};

    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use macros::{async_test_all, test_all};
    use platform_utils::time::SystemTime;
    use prost::Message as _;

    use super::{choose_pull_allowance, pull_transaction, read_pages};
    use crate::{
        Network,
        operator::rpc::spark_token::{
            BroadcastTransactionResponse, CommitStatus, FinalTokenOutput, FinalTokenTransaction,
            PartialTokenOutput, QueryTokenAllowancesRequest, QueryTokenAllowancesResponse,
            TokenAllowanceInfo, TokenAllowancePayload, TokenAllowanceStatus as Proto,
            TokenOutputToSpend, TokenTransferInput, final_token_transaction,
        },
        services::{ServiceError, TokenInputs, TokenTransaction, TokenTransactionStatus},
        token::TokenAllowanceFailure,
    };

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

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

    type Page = Result<QueryTokenAllowancesResponse, ServiceError>;

    fn page(allowances: Vec<TokenAllowanceInfo>, offset: i64) -> Page {
        Ok(QueryTokenAllowancesResponse { allowances, offset })
    }

    fn revoked(count: u8) -> Vec<TokenAllowanceInfo> {
        (0..count)
            .map(|tag| record(tag, Proto::Revoked, u64::from(tag)))
            .collect()
    }

    async fn paged(
        start: i64,
        row_cap: usize,
        pages: Vec<Page>,
    ) -> (
        Result<Vec<TokenAllowanceInfo>, ServiceError>,
        Vec<(i64, i64)>,
    ) {
        let sent = RefCell::new(Vec::new());
        let mut pages = pages.into_iter();
        let records = read_pages(
            &QueryTokenAllowancesRequest {
                offset: start,
                ..Default::default()
            },
            row_cap,
            |request| {
                sent.borrow_mut().push((request.offset, request.limit));
                ready(pages.next().expect("a page nobody expected"))
            },
        )
        .await;
        (records, sent.into_inner())
    }

    #[async_test_all]
    async fn the_newest_grant_on_the_second_page_is_found() {
        let (records, sent) = paged(
            0,
            usize::MAX,
            vec![
                page(revoked(100), 100),
                page(vec![record(200, Proto::Exhausted, 1_000)], -1),
            ],
        )
        .await;
        assert_eq!(sent, vec![(0, 100), (100, 100)]);
        assert_eq!(chosen(records.unwrap()), 200);
    }

    #[async_test_all]
    async fn paging_stops_at_the_last_page() {
        for last in [
            page(revoked(100), -1),
            page(revoked(99), 99),
            page(revoked(100), 0),
        ] {
            let (records, sent) = paged(0, usize::MAX, vec![last]).await;
            assert_eq!(sent, vec![(0, 100)]);
            assert!(records.is_ok());
        }
    }

    #[async_test_all]
    async fn paging_stops_at_the_row_cap() {
        let (records, sent) = paged(
            10,
            150,
            vec![page(revoked(100), 110), page(revoked(50), 160)],
        )
        .await;
        assert_eq!(sent, vec![(10, 100), (110, 50)]);
        assert_eq!(records.unwrap().len(), 150);

        let (records, sent) = paged(0, 0, vec![]).await;
        assert!(sent.is_empty());
        assert!(records.unwrap().is_empty());
    }

    #[async_test_all]
    async fn more_than_twenty_pages_are_refused() {
        let full_pages = || {
            (1..=20)
                .map(|n| page(revoked(100), n * 100))
                .collect::<Vec<_>>()
        };
        let (records, sent) = paged(0, usize::MAX, full_pages()).await;
        assert_eq!(sent.len(), 20);
        assert!(matches!(records, Err(ServiceError::InvalidInput(_))));

        let (records, _) = paged(0, 2_000, full_pages()).await;
        assert_eq!(records.unwrap().len(), 2_000);
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
