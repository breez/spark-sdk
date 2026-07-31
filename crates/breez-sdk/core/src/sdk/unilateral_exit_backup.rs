use serde::{Deserialize, Serialize};
use spark_wallet::{LeafPedigree, Network};
use tracing::{debug, warn};

use crate::{
    error::SdkError,
    models::{
        ExportUnilateralExitStateResponse, ImportUnilateralExitStateRequest,
        ImportUnilateralExitStateResponse,
    },
};

use super::BreezSdk;

const EXIT_STATE_VERSION: u32 = 1;

/// The exported payload. It embeds `LeafPedigree`'s own serde representation,
/// so `version` is what keeps an export readable once those internals change.
#[derive(Debug, Serialize, Deserialize)]
struct ExitStateEnvelope {
    version: u32,
    network: Network,
    identity_public_key: String,
    pedigrees: Vec<LeafPedigree>,
}

/// Rejects a payload this build cannot read at all: a version whose layout is
/// unknown, or another network's state.
fn check_envelope_readable(
    envelope: &ExitStateEnvelope,
    wallet_network: Network,
) -> Result<(), SdkError> {
    if envelope.version != EXIT_STATE_VERSION {
        return Err(SdkError::InvalidInput(format!(
            "Unsupported exit state version {}, expected {EXIT_STATE_VERSION}",
            envelope.version
        )));
    }
    if envelope.network != wallet_network {
        return Err(SdkError::InvalidInput(format!(
            "Exit state is for network {}, this wallet is on {wallet_network}",
            envelope.network
        )));
    }
    Ok(())
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    /// Serializes everything needed to unilaterally exit this wallet's funds
    /// while the Spark operators are unreachable, so it can be kept somewhere
    /// the wallet's own storage cannot take with it.
    ///
    /// The state goes stale as the wallet is used: export again whenever a
    /// `UnilateralExitStateChanged` event arrives.
    pub async fn export_unilateral_exit_state(
        &self,
    ) -> Result<ExportUnilateralExitStateResponse, SdkError> {
        let export = self.spark_wallet.export_exit_state().await?;
        let leaves = export.pedigrees.len();
        let envelope = ExitStateEnvelope {
            version: EXIT_STATE_VERSION,
            network: self.config.network.into(),
            identity_public_key: self.spark_wallet.get_identity_public_key().to_string(),
            pedigrees: export.pedigrees,
        };

        let exit_state = serde_json::to_string(&envelope)
            .map_err(|e| SdkError::Generic(format!("Failed to serialize exit state: {e}")))?;
        debug!(
            leaves,
            "export_unilateral_exit_state: exit state serialized"
        );

        Ok(ExportUnilateralExitStateResponse { exit_state })
    }

    /// Merges a previously exported exit state back into the wallet, without
    /// contacting the Spark operators. A leaf the exit state does not record
    /// this wallet as the owner of is skipped.
    ///
    /// A leaf the wallet can already exit keeps the data it has: an exit state
    /// carries no mark of when it was taken, so the imported copy is used only
    /// where the wallet has nothing that works. Importing an out of date state
    /// therefore never costs the wallet the ability to exit a leaf.
    ///
    /// The exit state must come from the same network the SDK is configured
    /// for.
    ///
    /// An out of date exit state can restore funds that have since been spent,
    /// so the balance may read high until the next sync reconciles it with the
    /// Spark operators.
    pub async fn import_unilateral_exit_state(
        &self,
        request: ImportUnilateralExitStateRequest,
    ) -> Result<ImportUnilateralExitStateResponse, SdkError> {
        let envelope: ExitStateEnvelope = serde_json::from_str(&request.exit_state)
            .map_err(|e| SdkError::InvalidInput(format!("Invalid exit state: {e}")))?;
        check_envelope_readable(&envelope, self.config.network.into())?;
        if envelope.identity_public_key != self.spark_wallet.get_identity_public_key().to_string() {
            // Not a rejection: the wallet filters leaf by leaf and reports what
            // it dropped.
            warn!(
                "Importing an exit state exported by another wallet: {}",
                envelope.identity_public_key
            );
        }

        let imported = self
            .spark_wallet
            .import_exit_state(envelope.pedigrees)
            .await?;
        let imported_leaves = u32::try_from(imported.imported_leaves)?;
        let skipped_leaves = u32::try_from(imported.skipped_foreign_leaves)?;
        let skipped_chains = u32::try_from(imported.skipped_chains)?;
        debug!(
            imported_leaves,
            skipped_leaves, skipped_chains, "import_unilateral_exit_state: exit state merged"
        );

        Ok(ImportUnilateralExitStateResponse {
            imported_leaves,
            skipped_leaves,
            skipped_chains,
        })
    }
}

#[cfg(test)]
mod tests {
    use spark_wallet::{TreeNodeStatus, tree_store_tests::create_test_node_with_parent};

    use super::*;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn regtest_envelope() -> ExitStateEnvelope {
        let root = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
        let leaf = create_test_node_with_parent("leaf", Some("root"), TreeNodeStatus::Available);
        let identity_public_key = leaf.owner_identity_public_key.unwrap().to_string();
        let pedigrees = vec![LeafPedigree {
            leaf,
            ancestors: vec![root],
        }];
        ExitStateEnvelope {
            version: EXIT_STATE_VERSION,
            network: Network::Regtest,
            identity_public_key,
            pedigrees,
        }
    }

    #[test]
    fn envelope_round_trips_through_json() {
        let envelope = regtest_envelope();
        let json = serde_json::to_string(&envelope).unwrap();
        let decoded: ExitStateEnvelope = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.version, envelope.version);
        assert_eq!(decoded.network, envelope.network);
        assert_eq!(decoded.identity_public_key, envelope.identity_public_key);
        assert_eq!(decoded.pedigrees.len(), 1);

        let pedigree = &decoded.pedigrees[0];
        let original = &envelope.pedigrees[0];
        assert_eq!(pedigree.leaf.id, original.leaf.id);
        assert_eq!(pedigree.leaf.parent_node_id, original.leaf.parent_node_id);
        assert_eq!(pedigree.leaf.node_tx, original.leaf.node_tx);
        assert_eq!(pedigree.leaf.value, original.leaf.value);
        let ancestor_ids: Vec<String> = pedigree
            .ancestors
            .iter()
            .map(|a| a.id.to_string())
            .collect();
        assert_eq!(ancestor_ids, vec!["root".to_string()]);

        check_envelope_readable(&decoded, Network::Regtest).unwrap();
    }

    #[test]
    fn envelope_with_unknown_version_is_rejected() {
        let mut envelope = regtest_envelope();
        envelope.version = EXIT_STATE_VERSION + 1;

        match check_envelope_readable(&envelope, Network::Regtest) {
            Err(SdkError::InvalidInput(message)) => assert!(
                message.contains("version"),
                "expected a version complaint, got {message}"
            ),
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn envelope_from_another_network_is_rejected() {
        let envelope = regtest_envelope();

        match check_envelope_readable(&envelope, Network::Mainnet) {
            Err(SdkError::InvalidInput(message)) => assert!(
                message.contains("network"),
                "expected a network complaint, got {message}"
            ),
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }
}
