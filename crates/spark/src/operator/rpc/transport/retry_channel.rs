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
use tonic::{Status, body::BoxBody, transport::Error as TransportError};
use tower_service::Service;
use tracing::{debug, trace};

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

impl<T, ResBody> Service<Request<BoxBody>> for RetryChannel<T>
where
    T: Service<Request<BoxBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    T::Future: Send + 'static,
    T::Error: std::error::Error + Send + Sync + 'static,
    ResBody: Body<Data = Bytes> + Send + 'static,
    <ResBody as Body>::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send,
{
    type Response = Response<ResBody>;
    type Error = T::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<BoxBody>) -> Self::Future {
        // Prepare the request body first
        let (head, mut body) = req.into_parts();
        let pinned_body = Pin::new(&mut body);
        let mut context = Context::from_waker(futures::task::noop_waker_ref());

        // Attempt to read the request, in order to create two copies of the request below.
        let poll = BoxBody::poll_frame(pinned_body, &mut context);
        let maybe_data = match poll {
            Poll::Ready(Some(Ok(frame))) => Some(frame.into_data().unwrap()),
            _ => None,
        };

        // Create two copies of the request if possible.
        let (original_req, maybe_retry_req) = if let Some(data) = maybe_data {
            let full_body =
                http_body_util::Full::new(data).map_err(|_| Status::internal("infallible error"));

            (
                Request::from_parts(head.clone(), BoxBody::new(full_body.clone())),
                Some(Request::from_parts(head, BoxBody::new(full_body))),
            )
        } else {
            // Can't create retry request, just use the original body
            (Request::from_parts(head, body), None)
        };

        // Clone the inner service for both initial call and potential retry
        let mut inner_clone_for_initial = self.inner.clone();
        let mut inner_clone_for_retry = self.inner.clone();

        Box::pin(async move {
            // Wait for the initial service to be ready and make the call
            poll_fn(|cx| inner_clone_for_initial.poll_ready(cx)).await?;
            let res = inner_clone_for_initial.call(original_req).await;

            // Early return if call succeeded or retry_req is None
            let retry_req = match (&res, maybe_retry_req) {
                (Ok(_), _) => return res,
                (_, None) => return res,
                (_, Some(req)) => req,
            };

            // Check if the error is a transport error
            match &res {
                Err(e) => {
                    // Use the standard error reflection pattern
                    let err_ref: &(dyn std::error::Error + 'static) =
                        e as &(dyn std::error::Error + 'static);
                    if !err_ref.is::<TransportError>() {
                        trace!("RetryChannel: non-transport error detected, not retrying");
                        return res;
                    }

                    debug!(
                        "RetryChannel: transport error detected: {:?}, retrying...",
                        e
                    );
                }
                _ => return res,
            };

            // Wait for the retry clone to be ready and make the call
            poll_fn(|cx| inner_clone_for_retry.poll_ready(cx)).await?;
            trace!("RetryChannel: making retry call");
            inner_clone_for_retry.call(retry_req).await
        })
    }
}
