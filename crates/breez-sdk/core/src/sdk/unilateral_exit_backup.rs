use bitcoin::{
    Transaction,
    consensus::serde::{Hex, With},
    secp256k1::PublicKey,
};
use serde::{Deserialize, Serialize};
use spark_wallet::{LeafPedigree, Network, SigningKeyshare, TreeNode, TreeNodeId, TreeNodeStatus};
use tracing::{debug, warn};

use crate::{
    error::SdkError,
    models::{
        ExportUnilateralExitStateResponse, ImportUnilateralExitStateRequest,
        ImportUnilateralExitStateResponse,
    },
};

use super::BreezSdk;

/// The layout exports are written in: transactions as consensus hex.
const EXIT_STATE_VERSION: u32 = 2;
/// The layout with transactions in rust-bitcoin's serde struct form. Still read:
/// exports taken in it stay valid backups.
const STRUCT_TX_EXIT_STATE_VERSION: u32 = 1;

/// The exported payload. Its pedigrees follow the SDK's internal node types, so
/// `version` is what keeps an export readable once those internals change.
#[derive(Debug, Serialize, Deserialize)]
struct ExitStateEnvelope<P> {
    version: u32,
    network: Network,
    identity_public_key: String,
    pedigrees: Vec<P>,
}

/// The field every layout shares, read first to pick the layout.
#[derive(Deserialize)]
struct ExitStateVersion {
    version: u32,
}

/// `TreeNode` as version 2 writes it: transactions as consensus hex, the form
/// bitcoin tooling decodes and broadcasts.
#[derive(Serialize, Deserialize)]
#[serde(remote = "TreeNode")]
struct HexTreeNode {
    id: TreeNodeId,
    tree_id: String,
    value: u64,
    parent_node_id: Option<TreeNodeId>,
    #[serde(with = "With::<Hex>")]
    node_tx: Transaction,
    #[serde(with = "optional_tx_hex")]
    refund_tx: Option<Transaction>,
    #[serde(with = "optional_tx_hex")]
    direct_tx: Option<Transaction>,
    #[serde(with = "optional_tx_hex")]
    direct_refund_tx: Option<Transaction>,
    #[serde(with = "optional_tx_hex")]
    direct_from_cpfp_refund_tx: Option<Transaction>,
    vout: u32,
    verifying_public_key: PublicKey,
    owner_identity_public_key: Option<PublicKey>,
    signing_keyshare: SigningKeyshare,
    status: TreeNodeStatus,
}

/// Lets a `Vec` hold `HexTreeNode`s: a remote definition only applies through a
/// field attribute.
#[derive(Serialize, Deserialize)]
struct HexNode(#[serde(with = "HexTreeNode")] TreeNode);

/// `LeafPedigree` as version 2 writes it.
#[derive(Serialize, Deserialize)]
struct HexLeafPedigree {
    leaf: HexNode,
    ancestors: Vec<HexNode>,
}

impl From<LeafPedigree> for HexLeafPedigree {
    fn from(LeafPedigree { leaf, ancestors }: LeafPedigree) -> Self {
        Self {
            leaf: HexNode(leaf),
            ancestors: ancestors.into_iter().map(HexNode).collect(),
        }
    }
}

impl From<HexLeafPedigree> for LeafPedigree {
    fn from(HexLeafPedigree { leaf, ancestors }: HexLeafPedigree) -> Self {
        Self {
            leaf: leaf.0,
            ancestors: ancestors.into_iter().map(|node| node.0).collect(),
        }
    }
}

mod optional_tx_hex {
    use bitcoin::{Transaction, consensus::encode};
    use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

    #[allow(clippy::ref_option)]
    pub(super) fn serialize<S: Serializer>(
        tx: &Option<Transaction>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        tx.as_ref().map(encode::serialize_hex).serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Transaction>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .map(|hex| encode::deserialize_hex(&hex).map_err(D::Error::custom))
            .transpose()
    }
}

fn encode_exit_state(
    network: Network,
    identity_public_key: String,
    pedigrees: Vec<LeafPedigree>,
) -> Result<String, SdkError> {
    let envelope = ExitStateEnvelope {
        version: EXIT_STATE_VERSION,
        network,
        identity_public_key,
        pedigrees: pedigrees.into_iter().map(HexLeafPedigree::from).collect(),
    };
    serde_json::to_string(&envelope)
        .map_err(|e| SdkError::Generic(format!("Failed to serialize exit state: {e}")))
}

/// Reads an exit state in any layout this build knows. Rejects one it cannot
/// use: an unknown version, or another network's state.
fn decode_exit_state(
    exit_state: &str,
    wallet_network: Network,
) -> Result<ExitStateEnvelope<LeafPedigree>, SdkError> {
    let ExitStateVersion { version } = parse_exit_state(exit_state)?;
    let envelope = match version {
        EXIT_STATE_VERSION => {
            let envelope: ExitStateEnvelope<HexLeafPedigree> = parse_exit_state(exit_state)?;
            ExitStateEnvelope {
                version: envelope.version,
                network: envelope.network,
                identity_public_key: envelope.identity_public_key,
                pedigrees: envelope
                    .pedigrees
                    .into_iter()
                    .map(LeafPedigree::from)
                    .collect(),
            }
        }
        STRUCT_TX_EXIT_STATE_VERSION => parse_exit_state(exit_state)?,
        _ => {
            return Err(SdkError::InvalidInput(format!(
                "Unsupported exit state version {version}"
            )));
        }
    };
    if envelope.network != wallet_network {
        return Err(SdkError::InvalidInput(format!(
            "Exit state is for network {}, this wallet is on {wallet_network}",
            envelope.network
        )));
    }
    Ok(envelope)
}

fn parse_exit_state<'a, T: Deserialize<'a>>(exit_state: &'a str) -> Result<T, SdkError> {
    serde_json::from_str(exit_state)
        .map_err(|e| SdkError::InvalidInput(format!("Invalid exit state: {e}")))
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
        let exit_state = encode_exit_state(
            self.config.network.into(),
            self.spark_wallet.get_identity_public_key().to_string(),
            export.pedigrees,
        )?;
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
        let envelope = decode_exit_state(&request.exit_state, self.config.network.into())?;
        if envelope.identity_public_key != self.spark_wallet.get_identity_public_key().to_string() {
            // Not a rejection: the wallet filters leaf by leaf and reports what
            // it dropped.
            warn!("Importing an exit state exported by another wallet");
            debug!("Exporting wallet: {}", envelope.identity_public_key);
        }

        let imported = self
            .spark_wallet
            .import_exit_state(envelope.pedigrees)
            .await?;
        let imported_leaves = u32::try_from(imported.imported_leaves)?;
        let skipped_foreign_leaves = u32::try_from(imported.skipped_foreign_leaves)?;
        let skipped_conflicting_leaves = u32::try_from(imported.skipped_conflicting_leaves)?;
        let skipped_chains = u32::try_from(imported.skipped_chains)?;
        debug!(
            imported_leaves,
            skipped_foreign_leaves,
            skipped_conflicting_leaves,
            skipped_chains,
            "import_unilateral_exit_state: exit state merged"
        );

        Ok(ImportUnilateralExitStateResponse {
            imported_leaves,
            skipped_foreign_leaves,
            skipped_conflicting_leaves,
            skipped_chains,
        })
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        Amount, OutPoint, ScriptBuf, Sequence, TxIn, TxOut, Txid, Witness, absolute::LockTime,
        consensus::encode::serialize_hex, hashes::Hash, transaction::Version,
    };
    use serde_json::Value;
    use spark_wallet::{TreeNodeStatus, tree_store_tests::create_test_node_with_parent};

    use super::*;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// Shaped like a pre-signed Spark transaction: a key path spend, so it
    /// carries a witness and takes the segwit encoding.
    fn signed_tx(sequence: u32) -> Transaction {
        Transaction {
            version: Version::non_standard(3),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::new(Txid::from_byte_array([1; 32]), 0),
                script_sig: ScriptBuf::new(),
                sequence: Sequence(sequence),
                witness: Witness::from_slice(&[[2u8; 64]]),
            }],
            output: vec![TxOut {
                value: Amount::from_sat(1_000),
                script_pubkey: ScriptBuf::new(),
            }],
        }
    }

    fn regtest_pedigree() -> LeafPedigree {
        let root = create_test_node_with_parent("root", None, TreeNodeStatus::Splitted);
        let mut leaf =
            create_test_node_with_parent("leaf", Some("root"), TreeNodeStatus::Available);
        leaf.node_tx = signed_tx(0);
        leaf.refund_tx = Some(signed_tx(1_900));
        LeafPedigree {
            leaf,
            ancestors: vec![root],
        }
    }

    fn identity_of(pedigree: &LeafPedigree) -> String {
        pedigree.leaf.owner_identity_public_key.unwrap().to_string()
    }

    fn encode_regtest(pedigree: &LeafPedigree) -> String {
        encode_exit_state(
            Network::Regtest,
            identity_of(pedigree),
            vec![pedigree.clone()],
        )
        .unwrap()
    }

    fn assert_same_pedigree(decoded: &LeafPedigree, original: &LeafPedigree) {
        assert_eq!(decoded.leaf, original.leaf);
        assert_eq!(decoded.ancestors, original.ancestors);
    }

    #[test]
    fn envelope_round_trips_through_json() {
        let pedigree = regtest_pedigree();
        let decoded = decode_exit_state(&encode_regtest(&pedigree), Network::Regtest).unwrap();

        assert_eq!(decoded.version, EXIT_STATE_VERSION);
        assert_eq!(decoded.network, Network::Regtest);
        assert_eq!(decoded.identity_public_key, identity_of(&pedigree));
        assert_eq!(decoded.pedigrees.len(), 1);
        assert_same_pedigree(&decoded.pedigrees[0], &pedigree);
    }

    #[test]
    fn envelope_writes_transactions_as_consensus_hex() {
        let pedigree = regtest_pedigree();
        let json: Value = serde_json::from_str(&encode_regtest(&pedigree)).unwrap();
        let leaf = &json["pedigrees"][0]["leaf"];
        let root = &json["pedigrees"][0]["ancestors"][0];

        assert_eq!(leaf["node_tx"], serialize_hex(&pedigree.leaf.node_tx));
        assert_eq!(
            leaf["refund_tx"],
            serialize_hex(pedigree.leaf.refund_tx.as_ref().unwrap())
        );
        assert_eq!(leaf["direct_tx"], Value::Null);
        assert_eq!(
            root["node_tx"],
            serialize_hex(&pedigree.ancestors[0].node_tx)
        );
    }

    #[test]
    fn envelope_with_struct_form_transactions_is_still_read() {
        let pedigree = regtest_pedigree();
        let exit_state = serde_json::to_string(&ExitStateEnvelope {
            version: STRUCT_TX_EXIT_STATE_VERSION,
            network: Network::Regtest,
            identity_public_key: identity_of(&pedigree),
            pedigrees: vec![pedigree.clone()],
        })
        .unwrap();
        let json: Value = serde_json::from_str(&exit_state).unwrap();
        assert!(json["pedigrees"][0]["leaf"]["node_tx"].is_object());

        let decoded = decode_exit_state(&exit_state, Network::Regtest).unwrap();

        assert_eq!(decoded.pedigrees.len(), 1);
        assert_same_pedigree(&decoded.pedigrees[0], &pedigree);
    }

    #[test]
    fn envelope_with_unknown_version_is_rejected() {
        let mut json: Value = serde_json::from_str(&encode_regtest(&regtest_pedigree())).unwrap();
        json["version"] = (EXIT_STATE_VERSION + 1).into();

        match decode_exit_state(&json.to_string(), Network::Regtest) {
            Err(SdkError::InvalidInput(message)) => assert!(
                message.contains("version"),
                "expected a version complaint, got {message}"
            ),
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn envelope_from_another_network_is_rejected() {
        match decode_exit_state(&encode_regtest(&regtest_pedigree()), Network::Mainnet) {
            Err(SdkError::InvalidInput(message)) => assert!(
                message.contains("network"),
                "expected a network complaint, got {message}"
            ),
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }
}
