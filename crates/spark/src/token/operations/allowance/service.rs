use std::sync::Arc;

use bitcoin::secp256k1::schnorr;
use platform_utils::time::{SystemTime, UNIX_EPOCH};
use tonic::Code;
use uuid::Uuid;

use crate::{
    Network,
    operator::{
        OperatorPool,
        rpc::{
            OperatorRpcError,
            spark_token::{
                CreateTokenAllowanceRequest, QueryTokenAllowancesRequest,
                QueryTokenAllowancesResponse, RevokeTokenAllowanceRequest, TokenAllowanceInfo,
            },
        },
    },
    services::ServiceError,
    signer::{PrepareTokenTransactionRequest, SparkSigner, TokenTransactionKind},
    token::{
        NewTokenAllowance, TokenAllowance, TokenAllowanceQuery, TokenAllowanceRole, TokensConfig,
        bech32m_decode_token_id,
    },
};

use super::{
    error::{classify_allowance_error, classify_revoke_error},
    hash::{hash_create_statement, hash_revoke_statement},
    model::revoke_payload,
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
            .map_err(|e| classify_revoke_error(e.into()))?;
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

    pub(super) async fn query(
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

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, future::ready};

    use macros::async_test_all;

    use super::read_pages;
    use crate::{
        operator::rpc::spark_token::{
            QueryTokenAllowancesRequest, QueryTokenAllowancesResponse, TokenAllowanceInfo,
            TokenAllowancePayload, TokenAllowanceStatus as Proto,
        },
        services::ServiceError,
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
        let newest = records.unwrap().pop().unwrap().allowance_payload.unwrap();
        assert_eq!(newest.allowance_id[0], 200);
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
}
