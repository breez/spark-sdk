use std::task::{Context, Poll};

use anyhow::{Result, anyhow};
use http::HeaderValue;
use platform_utils::ProxyConfig;
use tower_service::Service;

#[derive(Clone)]
pub struct Transport {
    inner: tonic_web_wasm_client::Client,
    user_agent: HeaderValue,
}

impl Service<http::Request<tonic::body::BoxBody>> for Transport {
    type Response = http::Response<tonic_web_wasm_client::ResponseBody>;
    type Error = tonic_web_wasm_client::Error;
    type Future =
        <tonic_web_wasm_client::Client as Service<http::Request<tonic::body::BoxBody>>>::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: http::Request<tonic::body::BoxBody>) -> Self::Future {
        // Set both `User-Agent` and `X-User-Agent`. Chrome silently drops any
        // script-set `User-Agent` on Fetch requests (crbug.com/571722), so we
        // also send `X-User-Agent` to ensure the value reaches the server
        // cross-browser. Firefox and Safari honor `User-Agent`.
        req.headers_mut()
            .insert("User-Agent", self.user_agent.clone());
        req.headers_mut()
            .insert("X-User-Agent", self.user_agent.clone());
        self.inner.call(req)
    }
}

#[derive(Clone)]
pub struct GrpcClient {
    inner: Transport,
}

impl GrpcClient {
    /// `proxy` must be `None` here: browser fetch offers no proxy control, so a
    /// proxied channel cannot be built. Callers are expected to have rejected
    /// the config already; this is the backstop that keeps an unhonoured proxy
    /// from turning into silent direct traffic.
    pub fn new(url: &str, user_agent: &str, proxy: Option<&ProxyConfig>) -> Result<Self> {
        if proxy.is_some() {
            return Err(anyhow!(
                "a SOCKS5 proxy cannot be honoured on WASM: fetch exposes no proxy control"
            ));
        }
        let user_agent = HeaderValue::from_str(user_agent)?;
        Ok(Self {
            inner: Transport {
                inner: tonic_web_wasm_client::Client::new(url.to_string()),
                user_agent,
            },
        })
    }

    pub fn into_inner(self) -> Transport {
        self.inner
    }
}
