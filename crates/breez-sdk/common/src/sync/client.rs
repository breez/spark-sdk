use anyhow::{Result, anyhow};

use tonic::{
    Request, Status, Streaming,
    metadata::{Ascii, MetadataValue},
    service::{Interceptor, interceptor::InterceptedService},
};

use crate::{
    grpc::transport::{GrpcClient, Transport},
    sync::proto::{
        GetLockReply, GetLockRequest, ListChangesReply, ListChangesRequest, ListenChangesRequest,
        Notification, SetLockReply, SetLockRequest, SetRecordReply, SetRecordRequest,
        syncer_client::SyncerClient as ProtoSyncerClient,
    },
};

#[cfg_attr(test, mockall::automock)]
#[macros::async_trait]
pub trait SyncerClient: Send + Sync {
    async fn set_record(&self, req: SetRecordRequest) -> Result<SetRecordReply>;
    async fn list_changes(&self, req: ListChangesRequest) -> Result<ListChangesReply>;
    async fn listen_changes(&self, req: ListenChangesRequest) -> Result<Streaming<Notification>>;
    async fn set_lock(&self, req: SetLockRequest) -> Result<SetLockReply>;
    async fn get_lock(&self, req: GetLockRequest) -> Result<GetLockReply>;
}

pub struct BreezSyncerClient {
    #[allow(unused)]
    client: ProtoSyncerClient<InterceptedService<Transport, ApiKeyInterceptor>>,
}

impl BreezSyncerClient {
    #[allow(unused)]
    pub fn new(server_url: &str, api_key: Option<&str>) -> anyhow::Result<Self> {
        let api_key_metadata = match &api_key {
            Some(key) => Some(
                format!("Bearer {key}")
                    .parse()
                    .map_err(|e| anyhow!("Invalid api key: {e}"))?,
            ),
            None => None,
        };

        let client = ProtoSyncerClient::with_interceptor(
            GrpcClient::new(server_url)?.into_inner(),
            ApiKeyInterceptor { api_key_metadata },
        );
        Ok(Self { client })
    }
}

#[macros::async_trait]
impl SyncerClient for BreezSyncerClient {
    async fn set_record(&self, req: SetRecordRequest) -> Result<SetRecordReply> {
        Ok(self.client.clone().set_record(req).await?.into_inner())
    }

    async fn list_changes(&self, req: ListChangesRequest) -> Result<ListChangesReply> {
        Ok(self.client.clone().list_changes(req).await?.into_inner())
    }

    async fn listen_changes(&self, req: ListenChangesRequest) -> Result<Streaming<Notification>> {
        Ok(self.client.clone().listen_changes(req).await?.into_inner())
    }

    async fn set_lock(&self, req: SetLockRequest) -> Result<SetLockReply> {
        Ok(self.client.clone().set_lock(req).await?.into_inner())
    }

    async fn get_lock(&self, req: GetLockRequest) -> Result<GetLockReply> {
        Ok(self.client.clone().get_lock(req).await?.into_inner())
    }
}

#[derive(Clone)]
pub struct ApiKeyInterceptor {
    api_key_metadata: Option<MetadataValue<Ascii>>,
}

impl Interceptor for ApiKeyInterceptor {
    fn call(&mut self, mut req: Request<()>) -> Result<Request<()>, Status> {
        if let Some(api_key_metadata) = &self.api_key_metadata {
            req.metadata_mut()
                .insert("authorization", api_key_metadata.clone());
        }
        Ok(req)
    }
}
