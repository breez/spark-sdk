use crate::operator_rpc::error::Result;
use crate::signer::Signer;
use spark_protos::spark::spark_service_client::SparkServiceClient;
use std::sync::Mutex;
use tonic::Request;
use tonic::Status;
use tonic::metadata::Ascii;
use tonic::metadata::MetadataValue;
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;

pub(crate) struct OperatorAuth<S>
where
    S: Signer,
{
    channel: Channel,
    signer: S,
    session: Mutex<Option<OperationSession>>,
}

#[derive(Clone)]
pub(crate) struct OperationSession {
    token: MetadataValue<Ascii>,
    expiration: u64,
}

impl<S> OperatorAuth<S>
where
    S: Signer,
{
    pub(crate) fn new(channel: Channel, signer: S) -> Self {
        Self {
            channel,
            signer,
            session: Mutex::new(None),
        }
    }

    pub(crate) fn spark_service_client(
        &self,
    ) -> Result<SparkServiceClient<InterceptedService<Channel, OperationSession>>> {
        let session = self.get_authenticated_session()?;
        Ok(SparkServiceClient::with_interceptor(
            self.channel.clone(),
            session,
        ))
    }

    pub(crate) fn get_authenticated_session(&self) -> Result<OperationSession> {
        let session = self.session.lock().unwrap();
        match session.as_ref() {
            Some(session) => Ok(session.clone()),
            None => {
                let session = self.authenticate()?;
                self.session.lock().unwrap().replace(session.clone());
                Ok(session)
            }
        }
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
