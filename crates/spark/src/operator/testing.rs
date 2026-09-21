//! Test helpers for code that holds an [`OperatorPool`].

use std::sync::Arc;

use frost_secp256k1_tr::Identifier;

use super::rpc::DefaultConnectionManager;
use super::{OperatorConfig, OperatorPool, OperatorPoolConfig};
use crate::session_store::InMemorySessionStore;
use crate::signer::SparkSigner;

/// A pool whose one operator listens nowhere, for code that needs a pool to
/// exist but is tested without calling it.
pub(crate) async fn unroutable_operator_pool(
    spark_signer: &Arc<dyn SparkSigner>,
) -> Arc<OperatorPool> {
    let config = OperatorPoolConfig::new(
        0,
        vec![OperatorConfig {
            id: 0,
            identifier: Identifier::try_from(1u16).unwrap(),
            address: "http://127.0.0.1:1".to_string(),
            ca_cert: None,
            identity_public_key: spark_signer.get_identity_public_key().await.unwrap(),
            user_agent: None,
        }],
    )
    .unwrap();
    Arc::new(
        OperatorPool::connect(
            &config,
            Arc::new(DefaultConnectionManager::new()),
            Arc::new(InMemorySessionStore::default()),
            Arc::clone(spark_signer),
            None,
        )
        .await
        .unwrap(),
    )
}
