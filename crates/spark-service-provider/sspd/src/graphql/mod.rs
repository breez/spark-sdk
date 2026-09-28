mod coop_exit;
mod lightning;
mod mutation;
mod query;
pub mod scalars;
mod static_deposit;
pub mod types;

use std::sync::{Arc, OnceLock};

use async_graphql::{Context, EmptySubscription, Error, Schema};
use async_graphql_axum::GraphQLResponse;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Router, routing::get, routing::post};
use bitcoin::secp256k1::PublicKey;

use crate::auth::AuthService;
use crate::coop_exit::CoopExitService;
use crate::lightning::receive::LightningReceiveService;
use crate::lightning::repository::LightningStore;
use crate::lightning::send::LightningSendService;
use crate::static_deposit::StaticDepositService;
use crate::swap::SwapService;

pub use mutation::MutationRoot;
pub use query::QueryRoot;
pub use types::BitcoinNetwork;

const MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;

const MAX_QUERY_DEPTH: usize = 16;

const MAX_QUERY_COMPLEXITY: usize = 2_000;

/// The cost of a field that calls bitcoind or the Lightning node, so that aliases
/// cannot repeat such calls more than a few times in one query.
const EXTERNAL_CALL_COMPLEXITY: usize = 100;

/// A cost that does not read the list of ids: a request's document holds every SDK
/// operation, and a cost is computed for operations whose variables it lacks.
const TRANSFERS_COMPLEXITY: usize = 500;

#[derive(Clone, Copy)]
pub struct AuthContext(pub Option<PublicKey>);

pub fn require_auth(ctx: &Context<'_>) -> Result<PublicKey, Error> {
    ctx.data_opt::<AuthContext>()
        .and_then(|c| c.0)
        .ok_or_else(|| Error::new("authentication required"))
}

pub type SspSchema = Schema<QueryRoot, MutationRoot, EmptySubscription>;

/// Enabled once the SSP's ldk-server is verified, which never happens without one.
#[derive(Clone, Default)]
pub struct Lightning(Arc<OnceLock<LightningServices>>);

pub struct LightningServices {
    pub send: Arc<LightningSendService>,
    pub receive: Arc<LightningReceiveService>,
}

impl Lightning {
    pub fn enable(&self, services: LightningServices) {
        let _ = self.0.set(services);
    }

    fn services<'a>(ctx: &Context<'a>) -> Result<&'a LightningServices, Error> {
        ctx.data::<Self>()?
            .0
            .get()
            .ok_or_else(|| Error::new("lightning is not available"))
    }
}

#[async_trait::async_trait]
pub trait RegtestFunder: Send + Sync {
    /// Returns the transaction id.
    async fn send_to_address(&self, address: &str, amount_sats: u64) -> Result<String, String>;
}

pub struct SchemaContext {
    pub swap_service: Arc<SwapService>,
    pub coop_exit_service: Arc<CoopExitService>,
    pub static_deposit_service: Arc<StaticDepositService>,
    pub lightning: Lightning,
    pub ln_store: Arc<dyn LightningStore>,
    pub network: BitcoinNetwork,
    pub auth: Arc<AuthService>,
    pub fee_rates: Arc<dyn crate::fees::FeeRateSource>,
    pub regtest_funder: Option<Arc<dyn RegtestFunder>>,
}

pub fn build_schema(context: SchemaContext) -> SspSchema {
    let builder = Schema::build(QueryRoot, MutationRoot, EmptySubscription)
        .limit_depth(MAX_QUERY_DEPTH)
        .limit_complexity(MAX_QUERY_COMPLEXITY)
        .data(context.swap_service)
        .data(context.coop_exit_service)
        .data(context.static_deposit_service)
        .data(context.ln_store)
        .data(context.network)
        .data(context.auth)
        .data(context.fee_rates)
        .data(context.regtest_funder)
        .data(context.lightning);
    builder.finish()
}

#[derive(Clone)]
struct AppState {
    schema: SspSchema,
    auth: Arc<AuthService>,
}

/// A token that does not validate is answered with a 401, which has the SDK sign
/// in again. A request without one runs unauthenticated, as signing in does.
async fn graphql_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let identity = match bearer_token(&headers) {
        None => None,
        Some(token) => match state
            .auth
            .validate_session(token, chrono::Utc::now().timestamp())
        {
            Ok(identity) => Some(identity),
            Err(_) => return StatusCode::UNAUTHORIZED.into_response(),
        },
    };
    let request: async_graphql::Request = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    // Run apart from the connection: a client that disconnects would otherwise
    // drop a resolver between steps that have to happen together, such as
    // reserving leaves and storing the request they belong to.
    let schema = state.schema.clone();
    match tokio::spawn(async move { schema.execute(request.data(AuthContext(identity))).await })
        .await
    {
        Ok(response) => GraphQLResponse::from(response).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

pub fn router(schema: SspSchema, auth: Arc<AuthService>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/graphql/{path}", post(graphql_handler))
        .route("/graphql/{path1}/{path2}", post(graphql_handler))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .with_state(AppState { schema, auth })
}

async fn health() -> &'static str {
    "ok"
}

#[cfg(test)]
mod tests {
    use async_graphql::{EmptySubscription, Request, Schema};

    use super::{MAX_QUERY_COMPLEXITY, MAX_QUERY_DEPTH, MutationRoot, QueryRoot};

    #[tokio::test]
    async fn sdk_operations_fit_the_query_limits() {
        let document = include_str!("../../../../spark/schema/queries.graphql");
        let schema = Schema::build(QueryRoot, MutationRoot, EmptySubscription)
            .limit_depth(MAX_QUERY_DEPTH)
            .limit_complexity(MAX_QUERY_COMPLEXITY)
            .finish();
        let parsed = async_graphql::parser::parse_query(document).expect("SDK operations parse");
        let names: Vec<String> = parsed
            .operations
            .iter()
            .filter_map(|(name, _)| name.map(ToString::to_string))
            .collect();
        assert!(!names.is_empty());
        for name in names {
            let response = schema
                .execute(Request::new(document).operation_name(&name))
                .await;
            // A resolver's error names its field, and a refused document names none.
            for error in response.errors {
                assert!(!error.path.is_empty(), "{name}: {}", error.message);
            }
        }
    }

    #[tokio::test]
    async fn a_list_longer_than_a_transfer_carries_is_refused() {
        let schema = Schema::build(QueryRoot, MutationRoot, EmptySubscription).finish();
        let query = "query($ids: [UUID!]!) { transfers(transfer_spark_ids: $ids) { __typename } }";
        let refused_for_length = |count: usize| {
            let ids: Vec<String> = (0..count)
                .map(|_| uuid::Uuid::new_v4().to_string())
                .collect();
            let request = Request::new(query).variables(async_graphql::Variables::from_json(
                serde_json::json!({ "ids": ids }),
            ));
            let schema = schema.clone();
            async move {
                schema
                    .execute(request)
                    .await
                    .errors
                    .iter()
                    .any(|error| error.message.contains("must be less than or equal to 1000"))
            }
        };
        assert!(!refused_for_length(1000).await);
        assert!(refused_for_length(1001).await);
    }

    #[tokio::test]
    async fn aliases_cannot_repeat_a_full_list_of_transfers_without_bound() {
        let schema = Schema::build(QueryRoot, MutationRoot, EmptySubscription)
            .limit_complexity(MAX_QUERY_COMPLEXITY)
            .finish();
        let ids: Vec<String> = (0..1000)
            .map(|_| uuid::Uuid::new_v4().to_string())
            .collect();
        let too_complex = |aliases: usize| {
            let fields = (0..aliases)
                .map(|n| format!("a{n}: transfers(transfer_spark_ids: $ids) {{ spark_id }}"))
                .collect::<Vec<_>>()
                .join(" ");
            let request = Request::new(format!("query($ids: [UUID!]!) {{ {fields} }}")).variables(
                async_graphql::Variables::from_json(serde_json::json!({ "ids": ids })),
            );
            let schema = schema.clone();
            async move {
                schema
                    .execute(request)
                    .await
                    .errors
                    .iter()
                    .any(|error| error.message.contains("too complex"))
            }
        };
        assert!(!too_complex(3).await);
        assert!(too_complex(4).await);
    }
}
