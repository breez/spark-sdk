use bitcoin::{Address, Transaction, consensus::encode::deserialize_hex, params::Params};
use spark_wallet::SparkWallet;

use crate::{command::DepositCommand, config::Config};

pub async fn handle_command<S>(
    config: &Config,
    wallet: &SparkWallet<S>,
    command: DepositCommand,
) -> Result<(), Box<dyn std::error::Error>>
where
    S: spark_wallet::Signer + Clone,
{
    match command {
        DepositCommand::NewAddress => {
            let address = wallet.generate_deposit_address(false).await?;
            println!("{}", address);
        }
        DepositCommand::NewAddressAndClaim => {
            let address = wallet.generate_deposit_address(false).await?;
            println!("{}", address);
            let mut rl = rustyline::DefaultEditor::new().unwrap();
            println!("Get funds from the faucet at https://app.lightspark.com/regtest-faucet");
            let line = rl.readline("paste txid> ")?;
            let txid = line.trim();
            let tx = get_transaction(&config, txid.to_string()).await?;
            let params: Params = config.spark_config.network.into();
            for (vout, output) in tx.output.iter().enumerate() {
                let Ok(output_address) = Address::from_script(&output.script_pubkey, &params)
                else {
                    continue;
                };

                if output_address != address {
                    continue;
                }

                let leaves = wallet.claim_deposit(tx, vout as u32).await?;
                println!(
                    "Claimed deposit: {}",
                    serde_json::to_string_pretty(&leaves)?
                );
                break;
            }
        }
        DepositCommand::Claim { txid } => {
            let tx = get_transaction(&config, txid).await?;
            // TODO: Look for correct output index
            let leaves = wallet.claim_deposit(tx, 0).await?;
            println!(
                "Claimed deposit: {}",
                serde_json::to_string_pretty(&leaves)?
            );
        }
    }

    Ok(())
}

async fn get_transaction(
    config: &Config,
    txid: String,
) -> Result<Transaction, Box<dyn std::error::Error>> {
    let url = format!("{}/tx/{}/hex", config.mempool_url, txid);
    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .basic_auth(
            config.mempool_username.clone(),
            Some(config.mempool_password.clone()),
        )
        .send()
        .await?;
    let hex = response.text().await?;
    let tx = deserialize_hex(&hex)?;
    Ok(tx)
}
