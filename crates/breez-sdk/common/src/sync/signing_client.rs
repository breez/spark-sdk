use std::{collections::HashMap, sync::Arc};

use bitcoin::{
    hashes::{Hash, sha256},
    hex::DisplayHex,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tonic::Streaming;
use tracing::trace;

use crate::{
    sync::{
        client::SyncerClient,
        model::{Record, RecordId},
        proto::{
            ListChangesRequest, ListenChangesRequest, Notification, SetRecordReply,
            SetRecordRequest,
        },
        signer::SyncSigner,
    },
    utils::now,
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
        let encrypted_data = self.signer.ecies_encrypt(serialized_data).await?;
        let msg = format!(
            "{}-{}-{}-{}-{}",
            record.id,
            serde_json::to_vec(&encrypted_data)?.to_lower_hex_string(),
            record.revision,
            record.schema_version,
            request_time,
        );
        let signature = self.sign_message(msg.as_bytes()).await?;
        let req = SetRecordRequest {
            client_id: Some(self.client_id.clone()),
            record: Some(crate::sync::proto::Record {
                id: record.id.to_string(),
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

    async fn sign_message(&self, msg: &[u8]) -> anyhow::Result<String> {
        let msg = [MESSAGE_PREFIX, msg].concat();
        trace!("About to compute sha256 hash of msg: {msg:?}");
        let digest = sha256::Hash::hash(&msg);
        trace!("About to sign digest: {digest:?}");
        self.signer
            .sign_ecdsa_recoverable(digest.as_byte_array())
            .await
            .map(|bytes| zbase32::encode_full_bytes(&bytes))
    }

    async fn map_record(&self, record: crate::sync::proto::Record) -> anyhow::Result<Record> {
        let decrypted = self.signer.ecies_decrypt(record.data).await?;
        let sync_data: SyncData = serde_json::from_slice(&decrypted)?;

        Ok(Record {
            id: sync_data.id,
            revision: record.revision,
            schema_version: record.schema_version.parse()?,
            data: sync_data.data,
        })
    }
}
