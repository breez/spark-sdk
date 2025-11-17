use std::str::FromStr;

use bitcoin::{
    self, PrivateKey, Psbt, Txid, Witness,
    consensus::encode::serialize_hex,
    ecdsa::Signature,
    key::Secp256k1,
    secp256k1::{PublicKey, SecretKey},
    sighash::SighashCache,
};
use clap::Subcommand;
use spark_wallet::{SparkWallet, SparkWalletError, TreeNodeId, is_ephemeral_anchor_output};

use crate::config::Config;

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

struct CpfpUtxo(spark_wallet::CpfpUtxo);

impl std::str::FromStr for CpfpUtxo {
    type Err = Box<dyn std::error::Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 4 {
            return Err("Invalid format, expected txid:vout:value:pubkey".into());
        }

        let txid = Txid::from_str(parts[0])?;
        let vout = parts[1].parse::<u32>()?;
        let value = parts[2].parse::<u64>()?;
        let pubkey_bytes = hex::decode(parts[3])?;
        let pubkey = PublicKey::from_slice(&pubkey_bytes)?;

        Ok(CpfpUtxo(spark_wallet::CpfpUtxo {
            txid,
            vout,
            value,
            pubkey,
        }))
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

pub async fn handle_command(
    config: &Config,
    wallet: &SparkWallet,
    command: WithdrawCommand,
) -> Result<(), Box<dyn std::error::Error>> {
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
                    None,
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
                .map(|s| CpfpUtxo::from_str(&s).map(|wrapper| wrapper.0))
                .collect::<Result<_, _>>()?;
            let all_leaf_tx_cpfp_psbts = wallet.unilateral_exit(fee_rate, leaf_ids, utxos).await?;

            for leaf_tx_cpfp_psbts in &all_leaf_tx_cpfp_psbts {
                println!();
                println!("Leaf ID: {}", leaf_tx_cpfp_psbts.leaf_id);
                println!();

                for (index, tx_cpfp_psbt) in leaf_tx_cpfp_psbts.tx_cpfp_psbts.iter().enumerate() {
                    let index_str = format!("{}. ", index + 1);
                    let index_spaces = " ".repeat(index_str.len());
                    let node_type = if index == leaf_tx_cpfp_psbts.tx_cpfp_psbts.len() - 1 {
                        "Leaf TX"
                    } else {
                        "Node TX"
                    };

                    let txid = tx_cpfp_psbt.parent_tx.compute_txid();
                    let tx_hex = serialize_hex(&tx_cpfp_psbt.parent_tx);
                    println!("{index_str}{node_type} ID: {txid}");
                    println!("{index_spaces}{node_type}: {tx_hex}");

                    let mut psbt = tx_cpfp_psbt.child_psbt.clone();
                    let psbt_hex = psbt.serialize_hex();
                    println!("{index_spaces}PSBT (unsigned): {psbt_hex}");

                    if let Some(signing_key) = &signing_key {
                        sign_psbt(&mut psbt, signing_key)?;

                        let signed_tx = psbt.extract_tx()?;
                        let signed_txid = signed_tx.compute_txid();
                        let signed_tx_hex = serialize_hex(&signed_tx);
                        println!("{index_spaces}PSBT signed TX ID: {signed_txid}");
                        println!("{index_spaces}PSBT signed TX: {signed_tx_hex}");
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
