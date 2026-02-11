use clap::Subcommand;
use serde::Serialize;
use spark_wallet::{SparkWallet, TreeNodeId};

#[derive(Clone, Debug, Subcommand)]
pub enum LeavesCommand {
    /// List all leaves in the wallet.
    List {
        /// Show compact output (id, tree_id, value, parent_node_id only)
        #[clap(short, long)]
        compact: bool,
    },
}

#[derive(Serialize)]
struct CompactLeaf {
    id: TreeNodeId,
    tree_id: String,
    value: u64,
    parent_node_id: Option<TreeNodeId>,
}

pub async fn handle_command(
    wallet: &SparkWallet,
    command: LeavesCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        LeavesCommand::List { compact } => {
            let leaves = wallet.list_leaves().await?;
            if compact {
                let compact_leaves: Vec<CompactLeaf> = leaves
                    .available
                    .into_iter()
                    .map(|leaf| CompactLeaf {
                        id: leaf.id,
                        tree_id: leaf.tree_id,
                        value: leaf.value,
                        parent_node_id: leaf.parent_node_id,
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&compact_leaves)?);
            } else {
                println!("{}", serde_json::to_string_pretty(&leaves)?);
            }
        }
    }

    Ok(())
}
