use bytes::Bytes;
use futures::future::poll_fn;
use http::{Request, Response};
use http_body::Body;
use http_body_util::BodyExt;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use tonic::Status;
use tower_service::Service;
use tracing::{debug, trace};

#[derive(Debug, thiserror::Error)]
pub enum RetryChannelError {
    #[error("gRPC status error: {0}")]
    Status(#[from] Status),
    #[error("transport error: {0}")]
    Transport(#[from] tonic::transport::Error),
}

/// A channel that retries a gRPC call once if a transport error occurs
#[derive(Debug, Clone)]
pub struct RetryChannel<T> {
    inner: T,
}

impl<T> RetryChannel<T>
where
    T: Clone,
{
    pub fn new(inner: T) -> Self {
        Self { inner }
    }
}

impl<T, ResBody> Service<Request<tonic::body::Body>> for RetryChannel<T>
where
    T: Service<
            Request<tonic::body::Body>,
            Response = Response<ResBody>,
            Error = tonic::transport::Error,
        > + Clone
        + Send
        + 'static,
    T::Future: Send + 'static,
    ResBody: Body<Data = Bytes> + Send + 'static,
    <ResBody as Body>::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send,
{
    type Response = Response<ResBody>;
    type Error = RetryChannelError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner
            .poll_ready(cx)
            .map_err(RetryChannelError::Transport)
    }

    fn call(&mut self, req: Request<tonic::body::Body>) -> Self::Future {
        // Clone the inner service for both initial call and potential retry
        let mut inner_clone = self.inner.clone();

        // Prepare the request body first
        let (head, body) = req.into_parts();

        Box::pin(async move {
            let data = body.collect().await?.to_bytes();
            let full_body = http_body_util::Full::new(data);
            let original_req =
                Request::from_parts(head.clone(), tonic::body::Body::new(full_body.clone()));
            let retry_req = Request::from_parts(head, tonic::body::Body::new(full_body));

            // Wait for the initial service to be ready and make the call
            poll_fn(|cx| inner_clone.poll_ready(cx)).await?;
            let res = inner_clone.call(original_req).await;

            let err = match res {
                Ok(ok) => return Ok(ok),
                Err(e) => e,
            };

            debug!(
                "RetryChannel: transport error detected: {:?}, retrying...",
                err
            );

            // Wait for the retry clone to be ready and make the call
            poll_fn(|cx| inner_clone.poll_ready(cx)).await?;
            trace!("RetryChannel: making retry call");
            Ok(inner_clone.call(retry_req).await?)
        })
    }
}
