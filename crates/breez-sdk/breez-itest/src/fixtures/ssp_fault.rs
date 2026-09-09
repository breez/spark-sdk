//! An SSP endpoint that can be told to fail a single GraphQL operation.
//!
//! Refusing the SSP wholesale is too blunt for testing an interrupted send:
//! `validate_payment` asks it for a fee estimate before the operators commit
//! anything, so the send would fail with nothing left behind and nothing to
//! resume. This forwards every request to the real SSP except the one operation
//! the test wants to lose.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tracing::info;

struct Proxy {
    upstream: String,
    operation: String,
    failing: AtomicBool,
    /// Let the SSP see the request and lose only its response, which is the
    /// ambiguous case: the send may or may not have been accepted.
    forward_before_failing: AtomicBool,
    /// Ids the SSP handed back for `operation`, in order. Two entries with the
    /// same id mean the SSP treated the second request as the first one again.
    request_ids: Mutex<Vec<String>>,
    client: reqwest::Client,
}

/// A local SSP that proxies to the real one. Stops when dropped, so keep it
/// alive for as long as the SDK under test points at it.
pub struct SspFaultProxy {
    proxy: Arc<Proxy>,
    base_url: String,
    server: JoinHandle<()>,
}

impl Drop for SspFaultProxy {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl SspFaultProxy {
    /// Proxies to `upstream`, ready to fail requests whose GraphQL body names
    /// `operation` (e.g. `"RequestLightningSend"`). Starts out passing
    /// everything through; call [`Self::start_failing`] to break the operation.
    pub async fn start(upstream: &str, operation: &str) -> Result<Self> {
        let proxy = Arc::new(Proxy {
            upstream: upstream.trim_end_matches('/').to_string(),
            operation: operation.to_string(),
            failing: AtomicBool::new(false),
            forward_before_failing: AtomicBool::new(false),
            request_ids: Mutex::new(Vec::new()),
            // The SDK's own client is the thing under test on the other side
            // of this proxy; this is the upstream hop and honours no proxy
            // config by design.
            #[allow(clippy::disallowed_methods)]
            client: reqwest::Client::new(),
        });

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base_url = format!("http://{}", listener.local_addr()?);
        let router = Router::new()
            .fallback(forward)
            .with_state(Arc::clone(&proxy));
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        info!("SSP fault proxy on {base_url} in front of {upstream}");
        Ok(Self {
            proxy,
            base_url,
            server,
        })
    }

    /// Put on `Config::spark_config`'s `ssp_config.base_url`.
    pub fn base_url(&self) -> String {
        self.base_url.clone()
    }

    /// Fail the operation without the SSP ever seeing it.
    pub fn start_failing(&self) {
        self.proxy
            .forward_before_failing
            .store(false, Ordering::SeqCst);
        self.proxy.failing.store(true, Ordering::SeqCst);
    }

    /// Let the SSP handle the operation, then lose the response.
    pub fn start_losing_responses(&self) {
        self.proxy
            .forward_before_failing
            .store(true, Ordering::SeqCst);
        self.proxy.failing.store(true, Ordering::SeqCst);
    }

    /// The ids the SSP returned for the watched operation, oldest first.
    pub fn request_ids(&self) -> Vec<String> {
        self.proxy
            .request_ids
            .lock()
            .expect("request id list is never poisoned")
            .clone()
    }

    pub fn stop_failing(&self) {
        self.proxy.failing.store(false, Ordering::SeqCst);
    }
}

async fn forward(
    State(proxy): State<Arc<Proxy>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Match operationName, not the body: the generated query string carries the
    // whole document, so every request mentions every operation in the file.
    let is_operation = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|body| {
            body.get("operationName")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .is_some_and(|name| name == proxy.operation);
    let fail = proxy.failing.load(Ordering::SeqCst) && is_operation;
    if fail && !proxy.forward_before_failing.load(Ordering::SeqCst) {
        info!(
            "SSP fault proxy failing {} before the SSP sees it",
            proxy.operation
        );
        return (StatusCode::SERVICE_UNAVAILABLE, "failed by test").into_response();
    }

    let url = format!("{}{}", proxy.upstream, uri.path());
    let mut request = proxy.client.post(&url).body(body);
    for (name, value) in &headers {
        // Host and length belong to the hop we just terminated.
        if name != axum::http::header::HOST && name != axum::http::header::CONTENT_LENGTH {
            request = request.header(name, value);
        }
    }

    match request.send().await {
        Ok(response) => {
            let status = response.status();
            match response.bytes().await {
                Ok(body) => {
                    if is_operation && let Some(id) = request_id(&body) {
                        proxy
                            .request_ids
                            .lock()
                            .expect("request id list is never poisoned")
                            .push(id);
                    }
                    if fail {
                        info!(
                            "SSP fault proxy losing the {} response the SSP just produced",
                            proxy.operation
                        );
                        return (StatusCode::SERVICE_UNAVAILABLE, "failed by test").into_response();
                    }
                    (status, body).into_response()
                }
                Err(e) => (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
            }
        }
        Err(e) => (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    }
}

/// Digs the request id out of a GraphQL response for the watched mutation,
/// wherever in the payload it sits.
fn request_id(body: &[u8]) -> Option<String> {
    fn find(value: &serde_json::Value) -> Option<String> {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(serde_json::Value::String(id)) = map.get("id")
                    && map.contains_key("status")
                {
                    return Some(id.clone());
                }
                map.values().find_map(find)
            }
            serde_json::Value::Array(items) => items.iter().find_map(find),
            _ => None,
        }
    }
    find(&serde_json::from_slice::<serde_json::Value>(body).ok()?)
}
