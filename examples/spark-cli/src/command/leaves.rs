use clap::Subcommand;
use spark_wallet::{SparkWallet, TreeNodeId};

use crate::config::Config;

#[derive(Clone, Debug, Subcommand)]
pub enum LeavesCommand {
    /// List all leaves in the wallet.
    List,

    Swap {
        #[clap(short, long, value_parser)]
        leaf_ids: Vec<TreeNodeId>,
        #[clap(short, long, value_parser)]
        target_amounts: Vec<u64>,
    },
}

pub async fn handle_command<S>(
    _config: &Config,
    wallet: &SparkWallet<S>,
    command: LeavesCommand,
) -> Result<(), Box<dyn std::error::Error>>
where
    S: spark_wallet::Signer + Clone,
{
    match command {
        LeavesCommand::List => {
            let leaves = wallet.list_leaves().await?;
            println!("{}", serde_json::to_string_pretty(&leaves)?);
        }

        LeavesCommand::Swap {
            leaf_ids,
            target_amounts,
        } => {
            let leaves = wallet.swap_leaves(leaf_ids, target_amounts).await?;
            println!("{}", serde_json::to_string_pretty(&leaves)?);
        }
    }

    Ok(())
}
