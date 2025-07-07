use spark_wallet::SparkWallet;

use crate::{command::TransferCommand, config::Config};

pub async fn handle_command<S>(
    config: &Config,
    wallet: &SparkWallet<S>,
    command: TransferCommand,
) -> Result<(), Box<dyn std::error::Error>>
where
    S: spark_wallet::Signer + Clone,
{
    match command {
        TransferCommand::ClaimPending => {
            let transfers = wallet.claim_pending_transfers().await?;
            println!(
                "Claimed transfers: {}",
                serde_json::to_string_pretty(&transfers)?
            );
        }
        TransferCommand::ListPending => {
            let transfers = wallet.list_pending_transfers().await?;
            println!(
                "Pending transfers: {}",
                serde_json::to_string_pretty(&transfers)?
            );
        }
        TransferCommand::Transfer {
            amount_sat,
            receiver_address,
        } => {
            let result = wallet.transfer(amount_sat, &receiver_address).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }

    Ok(())
}
