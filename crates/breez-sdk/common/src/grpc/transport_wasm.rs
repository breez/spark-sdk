use std::task::{Context, Poll};

use anyhow::Result;
use http::HeaderValue;
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
        req.headers_mut()
            .insert("User-Agent", self.user_agent.clone());
        self.inner.call(req)
    }
}

#[derive(Clone)]
pub struct GrpcClient {
    inner: Transport,
}

impl GrpcClient {
    pub fn new(url: &str, user_agent: &str) -> Result<Self> {
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
