use std::task::{Context, Poll};

use http::HeaderValue;
use tower_service::Service;

use crate::{default_user_agent, operator::rpc::OperatorRpcError};

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
    pub fn new(
        url: String,
        _ca_cert: Option<Vec<u8>>,
        user_agent: Option<String>,
    ) -> Result<Self, OperatorRpcError> {
        let user_agent = user_agent.unwrap_or_else(default_user_agent);
        let user_agent = HeaderValue::from_str(&user_agent)
            .map_err(|e| OperatorRpcError::Transport(e.to_string()))?;

        Ok(Self {
            inner: Transport {
                inner: tonic_web_wasm_client::Client::new(url),
                user_agent,
            },
        })
    }

    pub fn into_inner(self) -> Transport {
        self.inner
    }
}
