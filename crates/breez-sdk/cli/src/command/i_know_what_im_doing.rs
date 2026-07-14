use breez_sdk_spark::signer::single_key_cpfp_signer;
use breez_sdk_spark::{
    BreezSdk, ConfirmationStatus, CpfpFundingKind, CpfpInput, ExitLeafSelection,
    PrepareUnilateralExitRequest, UnilateralExitRequest, UnilateralExitResponse,
};
use clap::{Subcommand, ValueEnum};
use rustyline::{Editor, history::DefaultHistory};

use crate::command::{CliHelper, print_value};

#[derive(Clone, Copy, Debug, ValueEnum)]
#[clap(rename_all = "lower")]
pub enum FundingKindArg {
    P2wpkh,
    P2tr,
}

impl From<FundingKindArg> for CpfpFundingKind {
    fn from(kind: FundingKindArg) -> Self {
        match kind {
            FundingKindArg::P2wpkh => CpfpFundingKind::P2wpkh,
            FundingKindArg::P2tr => CpfpFundingKind::P2tr,
        }
    }
}

/// Expert-only commands that build raw transactions for you to broadcast
/// yourself. Misuse can strand or lose funds.
#[derive(Clone, Debug, Subcommand)]
pub enum IKnowWhatImDoingCommand {
    /// Build and sign a unilateral exit. Quotes it first (which leaves, fees, how
    /// much to fund), then prompts for the funding UTXOs and signing key.
    UnilateralExit {
        /// Target fee rate in sat/vByte.
        #[arg(long)]
        fee_rate: u64,
        /// Funding UTXO kind.
        #[arg(long, value_enum, default_value_t = FundingKindArg::P2tr)]
        funding_kind: FundingKindArg,
        /// Destination address for the swept funds.
        #[arg(long)]
        destination: String,
        /// Leaf id to exit (repeatable). Omit to auto-select every profitable leaf.
        #[arg(long = "leaf")]
        leaf_ids: Vec<String>,
    },
}

pub async fn handle_command(
    rl: &mut Editor<CliHelper, DefaultHistory>,
    sdk: &BreezSdk,
    command: IKnowWhatImDoingCommand,
) -> Result<bool, anyhow::Error> {
    match command {
        IKnowWhatImDoingCommand::UnilateralExit {
            fee_rate,
            funding_kind,
            destination,
            leaf_ids,
        } => {
            let prepared = sdk
                .prepare_unilateral_exit(PrepareUnilateralExitRequest {
                    fee_rate_sat_per_vbyte: fee_rate,
                    funding_kind: funding_kind.into(),
                    destination,
                    selection: exit_leaf_selection(leaf_ids),
                })
                .await?;
            print_value(&prepared)?;
            if prepared.leaves.is_empty() {
                println!("No leaves to exit.");
                return Ok(true);
            }

            let utxo_line = rl.readline(
                "Funding UTXO(s) as txid:vout:value:pubkey (space-separated, blank to stop): ",
            )?;
            if utxo_line.trim().is_empty() {
                println!("No funding provided; showing the quote only.");
                return Ok(true);
            }
            let funding_inputs = utxo_line
                .split_whitespace()
                .map(|u| parse_cpfp_input(u, funding_kind))
                .collect::<Result<Vec<_>, _>>()?;

            let key_line = rl.readline("Hex secret key for the funding UTXO(s): ")?;
            let signer = single_key_cpfp_signer(hex::decode(key_line.trim())?)?;

            let response = sdk
                .unilateral_exit(
                    UnilateralExitRequest {
                        prepared,
                        funding_inputs,
                    },
                    signer,
                )
                .await?;
            print_exit_transactions(&response);
            Ok(true)
        }
    }
}

/// Auto when no leaves are named, otherwise exactly the given leaves.
fn exit_leaf_selection(leaf_ids: Vec<String>) -> ExitLeafSelection {
    if leaf_ids.is_empty() {
        ExitLeafSelection::Auto
    } else {
        ExitLeafSelection::Specific { leaf_ids }
    }
}

/// Parses a `txid:vout:value:pubkey` funding UTXO into a [`CpfpInput`] of the
/// given kind. `pubkey` is hex; for P2TR it is the internal (untweaked) key.
fn parse_cpfp_input(s: &str, kind: FundingKindArg) -> Result<CpfpInput, anyhow::Error> {
    let [txid, vout, value, pubkey] = s.split(':').collect::<Vec<_>>()[..] else {
        return Err(anyhow::anyhow!(
            "invalid funding UTXO '{s}', expected txid:vout:value:pubkey"
        ));
    };
    let txid = txid.to_string();
    let vout = vout.parse::<u32>()?;
    let value = value.parse::<u64>()?;
    let pubkey = pubkey.to_string();
    Ok(match kind {
        FundingKindArg::P2wpkh => CpfpInput::P2wpkh {
            txid,
            vout,
            value,
            pubkey,
        },
        FundingKindArg::P2tr => CpfpInput::P2tr {
            txid,
            vout,
            value,
            pubkey,
        },
    })
}

/// Prints each exit transaction with a copy-pasteable `Package:` line: the tx
/// hex, plus its signed CPFP child when present, comma-separated. That is the
/// form mempool.space accepts for a package broadcast. Confirmed steps need no
/// broadcast, so they show no package.
fn print_exit_transactions(response: &UnilateralExitResponse) {
    println!(
        "Recoverable {} sats, total fee {} sats, {} transaction(s):",
        response.recoverable_value_sat,
        response.total_fee_sat,
        response.transactions.len(),
    );
    for (i, tx) in response.transactions.iter().enumerate() {
        let after = if tx.depends_on.is_empty() {
            String::new()
        } else {
            format!(", after {}", tx.depends_on.join(","))
        };
        let csv = tx
            .csv_timelock_blocks
            .map(|b| format!(", csv {b} blocks"))
            .unwrap_or_default();
        println!(
            "  [{i}] {:?} status={:?} txid={}{after}{csv}",
            tx.kind, tx.status, tx.txid,
        );
        if tx.status == ConfirmationStatus::Confirmed {
            println!("      (already confirmed, nothing to broadcast)");
            continue;
        }
        let package = match &tx.cpfp_tx_hex {
            Some(cpfp) => format!("{},{}", tx.tx_hex, cpfp),
            None => tx.tx_hex.clone(),
        };
        println!("      Package: {package}");
    }
}
