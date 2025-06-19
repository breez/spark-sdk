use crate::operator_rpc::error::Result;
use crate::signer::Signer;
use spark_protos::spark::spark_service_client::SparkServiceClient;
use tokio::sync::Mutex;
use tonic::Request;
use tonic::Status;
use tonic::metadata::Ascii;
use tonic::metadata::MetadataValue;
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;

pub struct OperatorAuth<S>
where
    S: Signer,
{
    channel: Channel,
    signer: S,
    session: Mutex<Option<OperationSession>>,
}

#[derive(Clone)]
pub struct OperationSession {
    token: MetadataValue<Ascii>,
    expiration: u64,
}

impl<S> OperatorAuth<S>
where
    S: Signer,
{
    pub fn new(channel: Channel, signer: S) -> Self {
        Self {
            channel,
            signer,
            session: Mutex::new(None),
        }
    }

    pub async fn spark_service_client(
        &self,
    ) -> Result<SparkServiceClient<InterceptedService<Channel, OperationSession>>> {
        let session = self.get_authenticated_session().await?;
        Ok(SparkServiceClient::with_interceptor(
            self.channel.clone(),
            session,
        ))
    }

    pub async fn get_authenticated_session(&self) -> Result<OperationSession> {
        if let Some(session) = self.session.lock().await.as_ref() {
            // Check if the session is still valid
            if session.expiration > tokio::time::Instant::now().elapsed().as_secs() {
                return Ok(session.clone());
            }
        }

        let session = self.authenticate()?;
        self.session.lock().await.replace(session.clone());
        Ok(session)
    }

    // TODO: implement authentication with rpc call
    fn authenticate(&self) -> Result<OperationSession> {
        todo!()
    }
}

impl Interceptor for OperationSession {
    fn call(&mut self, mut req: Request<()>) -> std::result::Result<Request<()>, Status> {
        req.metadata_mut()
            .insert("authorization", self.token.clone());
        Ok(req)
    }
}
