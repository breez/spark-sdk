use std::{collections::HashMap, sync::Arc};

use bitcoin::hex::DisplayHex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tonic::Streaming;

use crate::{
    sync::{
        client::SyncerClient,
        model::{Record, RecordId},
        proto::{
            GetLockRequest, ListChangesRequest, ListenChangesRequest, Notification, SetLockRequest,
            SetRecordReply, SetRecordRequest,
        },
        signer::SyncSigner,
    },
    utils::{now, zbase32::encode_zbase32},
};

const MESSAGE_PREFIX: &[u8; 13] = b"realtimesync:";

#[derive(Deserialize, Serialize)]
struct SyncData {
    id: RecordId,
    data: HashMap<String, Value>,
}

impl SyncData {
    pub fn new(record: Record) -> Self {
        SyncData {
            id: record.id,
            data: record.data,
        }
    }
}

#[derive(Clone)]
pub struct SigningClient {
    inner: Arc<dyn SyncerClient>,
    signer: Arc<dyn SyncSigner>,
    pub client_id: String,
}

impl SigningClient {
    pub fn new(
        inner: Arc<dyn SyncerClient>,
        signer: Arc<dyn SyncSigner>,
        client_id: String,
    ) -> Self {
        SigningClient {
            inner,
            signer,
            client_id,
        }
    }

    pub async fn set_record(&self, record: &Record) -> anyhow::Result<SetRecordReply> {
        let request_time: u32 = now();
        let serialized_data = serde_json::to_vec(&SyncData::new(record.clone()))?;
        let encrypted_data = self.signer.encrypt_ecies(serialized_data).await?;
        let msg = format!(
            "{}-{}-{}-{}-{}",
            record.id.to_id_string(),
            encrypted_data.to_lower_hex_string(),
            record.revision,
            record.schema_version,
            request_time,
        );
        let signature = self.sign_message(msg.as_bytes()).await?;
        let req = SetRecordRequest {
            client_id: Some(self.client_id.clone()),
            record: Some(crate::sync::proto::Record {
                id: record.id.to_id_string(),
                revision: record.revision,
                schema_version: record.schema_version.to_string(),
                data: encrypted_data,
            }),
            request_time,
            signature,
        };
        self.inner.set_record(req).await
    }

    pub async fn list_changes(&self, since_revision: u64) -> anyhow::Result<Vec<Record>> {
        let request_time = now();
        let msg = format!("{since_revision}-{request_time}");
        let signature = self.sign_message(msg.as_bytes()).await?;
        let request = ListChangesRequest {
            since_revision,
            request_time,
            signature,
        };

        let reply = self.inner.list_changes(request).await?;
        let mut changes = Vec::new();
        for change in reply.changes {
            changes.push(self.map_record(change).await?);
        }
        Ok(changes)
    }

    pub async fn listen_changes(&self) -> anyhow::Result<Streaming<Notification>> {
        let request_time = now();
        let msg = format!("{request_time}");
        let signature = self.sign_message(msg.as_bytes()).await?;
        let request = ListenChangesRequest {
            request_time,
            signature,
        };

        let stream = self.inner.listen_changes(request).await?;
        Ok(stream)
    }

    pub async fn set_lock(&self, params: crate::sync::SetLockParams) -> anyhow::Result<()> {
        let request_time: u32 = now();
        let instance_id = &self.client_id;
        let msg = format!(
            "{}-{instance_id}-{}-{}-{request_time}",
            params.lock_name, params.acquire, params.exclusive
        );
        let signature = self.sign_message(msg.as_bytes()).await?;
        let req = SetLockRequest {
            lock_name: params.lock_name,
            instance_id: self.client_id.clone(),
            acquire: params.acquire,
            exclusive: params.exclusive,
            ttl_seconds: None,
            request_time,
            signature,
        };
        self.inner.set_lock(req).await?;
        Ok(())
    }

    pub async fn get_lock(&self, lock_name: &str) -> anyhow::Result<bool> {
        let request_time: u32 = now();
        let msg = format!("{lock_name}-{request_time}");
        let signature = self.sign_message(msg.as_bytes()).await?;
        let req = GetLockRequest {
            lock_name: lock_name.to_string(),
            request_time,
            signature,
        };
        let reply = self.inner.get_lock(req).await?;
        Ok(reply.locked)
    }

    async fn sign_message(&self, msg: &[u8]) -> anyhow::Result<String> {
        let msg = [MESSAGE_PREFIX, msg].concat();
        self.signer
            .sign_ecdsa_recoverable(&msg)
            .await
            .map(|bytes| encode_zbase32(&bytes))
    }

    async fn map_record(&self, record: crate::sync::proto::Record) -> anyhow::Result<Record> {
        let decrypted = self.signer.decrypt_ecies(record.data).await?;
        let sync_data: SyncData = serde_json::from_slice(&decrypted)?;

        Ok(Record {
            id: sync_data.id,
            revision: record.revision,
            schema_version: record.schema_version.parse().map_err(anyhow::Error::msg)?,
            data: sync_data.data,
        })
    }
}
