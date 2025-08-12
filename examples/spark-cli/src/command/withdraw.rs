use std::str::FromStr;

use bitcoin::{
    self, PrivateKey, Psbt, Witness, consensus::encode::serialize_hex, ecdsa::Signature,
    key::Secp256k1, secp256k1::SecretKey, sighash::SighashCache,
};
use clap::Subcommand;
use spark_wallet::{
    FeeBumpUtxo, SparkWallet, SparkWalletError, TreeNodeId, is_ephemeral_anchor_output,
};

use crate::{config::Config, mempool::get_transaction};

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum ExitSpeed {
    Fast,
    Medium,
    Slow,
}

impl From<ExitSpeed> for spark_wallet::ExitSpeed {
    fn from(speed: ExitSpeed) -> Self {
        match speed {
            ExitSpeed::Fast => spark_wallet::ExitSpeed::Fast,
            ExitSpeed::Medium => spark_wallet::ExitSpeed::Medium,
            ExitSpeed::Slow => spark_wallet::ExitSpeed::Slow,
        }
    }
}

#[derive(Clone, Debug, Subcommand)]
pub enum WithdrawCommand {
    /// Fetch the current coop exit fee quote.
    FetchFeeQuote {
        withdrawal_address: String,
        amount_sats: Option<u64>,
    },
    /// Perform a coop exit.
    CoopExit {
        withdrawal_address: String,
        exit_speed: ExitSpeed,
        amount_sats: Option<u64>,
    },
    /// Prepare a unilateral exit package.
    UnilateralExit {
        /// Fee rate in sats/vbyte.
        fee_rate: u64,
        /// The leaf IDs of the tree nodes to unilateral exit. Defaults to all leaves.
        #[clap(short, long = "leaf")]
        leaf_ids: Vec<TreeNodeId>,
        /// Hex-encoded UTXOs "[txid:vout:value:pubkey]" used to pay fees for the unilateral exit.
        #[clap(short, long = "utxo")]
        utxos: Vec<String>,
        /// Optional hex-encoded private key to sign PSBTs
        #[clap(short)]
        signing_key: Option<String>,
    },
}

pub async fn handle_command<S>(
    config: &Config,
    wallet: &SparkWallet<S>,
    command: WithdrawCommand,
) -> Result<(), Box<dyn std::error::Error>>
where
    S: spark_wallet::Signer + Clone,
{
    match command {
        WithdrawCommand::FetchFeeQuote {
            withdrawal_address,
            amount_sats,
        } => {
            let fee_quote = wallet
                .fetch_coop_exit_fee_quote(&withdrawal_address, amount_sats)
                .await?;
            println!("{}", serde_json::to_string_pretty(&fee_quote)?);
        }
        WithdrawCommand::CoopExit {
            withdrawal_address,
            exit_speed,
            amount_sats,
        } => {
            let fee_quote = wallet
                .fetch_coop_exit_fee_quote(&withdrawal_address, amount_sats)
                .await?;

            let result = wallet
                .withdraw(
                    &withdrawal_address,
                    amount_sats,
                    exit_speed.into(),
                    fee_quote,
                )
                .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        WithdrawCommand::UnilateralExit {
            fee_rate,
            leaf_ids,
            utxos,
            signing_key,
        } => {
            let signing_key = signing_key
                .map(|pk| SecretKey::from_str(&pk))
                .transpose()?
                .map(|pk| PrivateKey::new(pk, config.spark_config.network));

            let utxos = utxos
                .into_iter()
                .map(|s| FeeBumpUtxo::from_str(&s))
                .collect::<Result<_, _>>()?;
            let all_leaf_tx_fee_bump_psbts = wallet
                .unilateral_exit(fee_rate, leaf_ids, utxos, |txid| async move {
                    get_transaction(config, txid)
                        .await
                        .map_err(|e| SparkWalletError::Generic(e.to_string()))
                })
                .await?;

            for leaf_tx_fee_bump_psbts in &all_leaf_tx_fee_bump_psbts {
                println!();
                println!("Leaf ID: {}", leaf_tx_fee_bump_psbts.leaf_id);
                println!();

                for (index, tx_fee_bump_psbt) in
                    leaf_tx_fee_bump_psbts.tx_fee_bump_psbts.iter().enumerate()
                {
                    let node_type = if index == leaf_tx_fee_bump_psbts.tx_fee_bump_psbts.len() - 1 {
                        "Leaf TX"
                    } else {
                        "Node TX"
                    };

                    let tx_hex = serialize_hex(&tx_fee_bump_psbt.tx);
                    println!("{}. {}: {}", index + 1, node_type, tx_hex);

                    let mut psbt = tx_fee_bump_psbt.psbt.clone();
                    let psbt_hex = psbt.serialize_hex();
                    println!("{}. PSBT (unsigned): {}", index + 1, psbt_hex);
                    if let Some(signing_key) = &signing_key {
                        sign_psbt(&mut psbt, signing_key)?;
                        let signed_tx_hex = serialize_hex(&psbt.extract_tx()?);
                        println!("{}. PSBT signed TX: {}", index + 1, signed_tx_hex);
                    }
                    println!();
                }
                println!();
            }
            println!("For each leaf, broadcast one-by-one each TX and signed PSBT.");
            println!(
                "TXs and signed PSBTs should be broadcasted in the order they appear: Node(s) > Leaf"
            );
        }
    }

    Ok(())
}

fn sign_psbt(psbt: &mut Psbt, signing_key: &PrivateKey) -> Result<(), SparkWalletError> {
    let mut cache = SighashCache::new(&psbt.unsigned_tx);
    let mut signatures = vec![];
    let mut anchor_index = None;
    let secp = Secp256k1::new();
    let pubkey = signing_key.public_key(&secp);
    // Sign inputs where the witness utxo is a non anchor output
    for (i, input) in psbt.inputs.iter().enumerate() {
        if let Some(tx_out) = &input.witness_utxo {
            if is_ephemeral_anchor_output(tx_out) {
                anchor_index = Some(i);
            } else {
                let (msg, ecdsa_type) = psbt
                    .sighash_ecdsa(i, &mut cache)
                    .map_err(|e| SparkWalletError::Generic(e.to_string()))?;
                let sig = secp.sign_ecdsa(&msg, &signing_key.inner);
                let signature = Signature {
                    signature: sig,
                    sighash_type: ecdsa_type,
                };
                signatures.push((i, pubkey, signature));
            }
        }
    }
    // Update the inputs partial sigs with the signatures
    for (i, pubkey, signature) in signatures.into_iter() {
        psbt.inputs[i].partial_sigs.insert(pubkey, signature);
    }
    // Set an empty witness for the anchor input
    if let Some(anchor_index) = anchor_index {
        psbt.inputs[anchor_index].final_script_witness = Some(Witness::new())
    }
    Ok(())
}
