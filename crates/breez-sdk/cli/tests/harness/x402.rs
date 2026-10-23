//! An HTTP resource charged for with x402, served in-process by its own SDK on
//! the scenario network. The SDK only receives, so it needs no funds.

use std::sync::Arc;

use anyhow::Result;
use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{AppendHeaders, IntoResponse, Response},
    routing::any,
};
use breez_sdk_spark::{
    BreezSdk, ConnectRequest, CreateX402ChallengeRequest, Network, Seed, SettleX402Request, X402,
    X402Header, X402PaymentAmount, X402Request, connect, default_config,
};

const PRICE_SATS: u64 = 100;
const CONTENT: &str = r#"{"content":"paid article"}"#;
const BOUND_NAMES: [&str; 1] = ["accept"];
/// The payment may still settle, so the client should send it again.
const RETRYABLE_REASONS: [&str; 2] = [
    "exact_lnbtc_invoice_not_settled",
    "exact_lnbtc_receiver_unavailable",
];

pub struct X402Fixture {
    pub url: String,
    server: tokio::task::JoinHandle<()>,
    _sdk: BreezSdk,
    _dir: tempfile::TempDir,
}

impl X402Fixture {
    pub async fn start() -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let mut config = default_config(Network::Regtest);
        config.api_key = std::env::var("BREEZ_API_KEY").ok();
        let sdk = connect(ConnectRequest {
            config,
            seed: Seed::Mnemonic {
                mnemonic: bip39::Mnemonic::generate(12)?.to_string(),
                passphrase: None,
            },
            storage_dir: dir.path().to_string_lossy().into_owned(),
        })
        .await?;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let charger = Arc::new(Charger {
            x402: sdk.get_x402(),
            origin: origin.clone(),
        });
        let app = Router::new()
            .route("/article", any(serve))
            .with_state(charger);
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self {
            url: format!("{origin}/article"),
            server,
            _sdk: sdk,
            _dir: dir,
        })
    }
}

impl Drop for X402Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

struct Charger {
    x402: X402,
    origin: String,
}

async fn serve(
    State(charger): State<Arc<Charger>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    charger
        .charge(&method, &uri, &headers, &body)
        .await
        .unwrap_or_else(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response())
}

impl Charger {
    async fn charge(
        &self,
        method: &Method,
        uri: &Uri,
        headers: &HeaderMap,
        body: &Bytes,
    ) -> Result<Response> {
        let target = uri.path_and_query().map_or("/", |p| p.as_str());
        let request = X402Request::Http {
            method: method.to_string(),
            url: format!("{}{target}", self.origin),
            headers: headers
                .iter()
                .filter_map(|(name, value)| {
                    Some(X402Header {
                        name: name.to_string(),
                        value: value.to_str().ok()?.to_string(),
                    })
                })
                .collect(),
            body: (!body.is_empty()).then(|| body.to_vec()),
        };
        let bound_names: Vec<String> = BOUND_NAMES.iter().map(ToString::to_string).collect();

        let mut response_headers = Vec::new();
        if let Some(payment) = headers
            .get("payment-signature")
            .and_then(|v| v.to_str().ok())
        {
            let settled = self
                .x402
                .settle(SettleX402Request {
                    payment: payment.to_string(),
                    request: request.clone(),
                    bound_names: bound_names.clone(),
                })
                .await?;
            response_headers.push(("payment-response", settled.payment_response));
            if settled.success {
                response_headers.push(("content-type", "application/json".to_string()));
                return Ok((AppendHeaders(response_headers), CONTENT).into_response());
            }
            if settled
                .error_reason
                .as_deref()
                .is_some_and(|reason| RETRYABLE_REASONS.contains(&reason))
            {
                return Ok((
                    StatusCode::PAYMENT_REQUIRED,
                    AppendHeaders(response_headers),
                )
                    .into_response());
            }
        }

        let challenge = self
            .x402
            .create_challenge(CreateX402ChallengeRequest {
                request,
                bound_names,
                amounts: vec![X402PaymentAmount::Lightning {
                    amount_sats: PRICE_SATS,
                }],
                max_timeout_secs: None,
                description: Some("A paid article".to_string()),
                mime_type: Some("application/json".to_string()),
            })
            .await?;
        response_headers.push(("payment-required", challenge.payment_required));
        Ok((
            StatusCode::PAYMENT_REQUIRED,
            AppendHeaders(response_headers),
        )
            .into_response())
    }
}
