use spark_wallet::SparkWallet;

use crate::{command::LightningCommand, config::Config};

pub async fn handle_command<S>(
    _config: &Config,
    wallet: &SparkWallet<S>,
    command: LightningCommand,
) -> Result<(), Box<dyn std::error::Error>>
where
    S: spark_wallet::Signer + Clone,
{
    match command {
        LightningCommand::CreateInvoice {
            amount_sat,
            description,
        } => {
            let payment = wallet
                .create_lightning_invoice(amount_sat, description)
                .await?;
            println!("{}", serde_json::to_string_pretty(&payment)?);
        }
        LightningCommand::FetchReceivePayment { id } => {
            let payment = wallet.fetch_lightning_receive_payment(&id).await?;
            println!("{}", serde_json::to_string_pretty(&payment)?);
        }
        LightningCommand::FetchSendFeeEstimate { invoice } => {
            let fee = wallet.fetch_lightning_send_fee_estimate(&invoice).await?;
            println!("{}", fee);
        }
        LightningCommand::FetchSendPayment { id } => {
            let payment = wallet.fetch_lightning_send_payment(&id).await?;
            println!("{}", serde_json::to_string_pretty(&payment)?);
        }
        LightningCommand::PayInvoice {
            invoice,
            max_fee_sat,
        } => {
            let payment = wallet.pay_lightning_invoice(&invoice, max_fee_sat).await?;
            println!("{}", serde_json::to_string_pretty(&payment)?);
        }
    }

    Ok(())
}
