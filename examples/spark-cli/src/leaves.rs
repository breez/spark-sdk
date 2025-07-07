use spark_wallet::SparkWallet;

use crate::{command::LeavesCommand, config::Config};

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
    }

    Ok(())
}
