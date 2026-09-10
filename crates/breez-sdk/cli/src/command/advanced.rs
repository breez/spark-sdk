use std::fs;
use std::path::{Path, PathBuf};

use breez_sdk_spark::signer::single_key_cpfp_signer;
use breez_sdk_spark::{
    BreezSdk, CheckUnilateralExitRequest, CpfpFundingKind, CpfpInput, ExitLeafSelection,
    ExitTransactionStatus, ImportUnilateralExitStateRequest, PrepareUnilateralExitRequest,
    UnilateralExitRequest, UnilateralExitResponse, UnilateralExitVerdict,
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
pub enum AdvancedCommand {
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
        /// File to write the signed exit to, for `check-unilateral-exit` to read
        /// back once its transactions are on-chain.
        #[arg(long)]
        output_file: Option<PathBuf>,
    },
    /// Read a signed exit written by `unilateral-exit` back against the chain:
    /// which of its transactions confirmed, what is ready to broadcast now, and
    /// whether the exit still holds.
    CheckUnilateralExit {
        /// File the exit was written to.
        #[arg(long)]
        input_file: PathBuf,
        /// File to write the updated exit to. Defaults to `--input-file`.
        #[arg(long)]
        output_file: Option<PathBuf>,
    },
    /// Export the wallet's unilateral exit state to a file, for safekeeping
    /// outside the wallet's own storage.
    ExportUnilateralExitState {
        /// File to write the exit state to.
        #[arg(long)]
        output_file: PathBuf,
    },
    /// Import a unilateral exit state previously written by
    /// `export-unilateral-exit-state`, merging it into the wallet.
    ImportUnilateralExitState {
        /// File the exit state was exported to.
        #[arg(long)]
        input_file: PathBuf,
    },
}

pub async fn handle_command(
    rl: &mut Editor<CliHelper, DefaultHistory>,
    sdk: &BreezSdk,
    command: AdvancedCommand,
) -> Result<bool, anyhow::Error> {
    match command {
        AdvancedCommand::UnilateralExit {
            fee_rate,
            funding_kind,
            destination,
            leaf_ids,
            output_file,
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
            if let Some(output_file) = output_file {
                write_exit(&output_file, &response)?;
            }
            Ok(true)
        }
        AdvancedCommand::CheckUnilateralExit {
            input_file,
            output_file,
        } => {
            let exit = read_exit(&input_file)?;
            let checked = sdk
                .check_unilateral_exit(CheckUnilateralExitRequest { exit })
                .await?;
            println!("Verdict: {:?}", checked.verdict);
            if let UnilateralExitVerdict::Redo { .. } = checked.verdict {
                println!("  (this exit cannot be finished, quote and build it again)");
            }
            print_exit_transactions(&checked.exit);
            write_exit(output_file.as_deref().unwrap_or(&input_file), &checked.exit)?;
            Ok(true)
        }
        AdvancedCommand::ExportUnilateralExitState { output_file } => {
            let exported = sdk.export_unilateral_exit_state().await?;
            fs::write(&output_file, &exported.exit_state)?;
            println!(
                "Wrote {} bytes to {}",
                exported.exit_state.len(),
                output_file.display(),
            );
            Ok(true)
        }
        AdvancedCommand::ImportUnilateralExitState { input_file } => {
            let exit_state = fs::read_to_string(&input_file)?;
            let imported = sdk
                .import_unilateral_exit_state(ImportUnilateralExitStateRequest { exit_state })
                .await?;
            println!(
                "Imported {} leaf(s), skipped {} leaf(s) from a different wallet \
                 and {} that disagree with what this wallet holds, \
                 left out the exit data of {} leaf(s)",
                imported.imported_leaves,
                imported.skipped_foreign_leaves,
                imported.skipped_conflicting_leaves,
                imported.skipped_chains,
            );
            Ok(true)
        }
    }
}

/// Reads an exit written by [`write_exit`].
fn read_exit(path: &Path) -> Result<UnilateralExitResponse, anyhow::Error> {
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

/// Writes the exit as JSON, the form [`read_exit`] reads back.
fn write_exit(path: &Path, exit: &UnilateralExitResponse) -> Result<(), anyhow::Error> {
    fs::write(path, serde_json::to_string_pretty(exit)?)?;
    println!("Wrote the exit to {}", path.display());
    Ok(())
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
        "Recoverable {} sats, total fee {} sats (cpfp {}, fanout {}, sweep {}), \
         {} transaction(s):",
        response.recoverable_value_sat,
        response.total_fee_sat,
        response.cpfp_fee_sat,
        response.fanout_fee_sat,
        response.sweep_fee_sat,
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
        match tx.status {
            ExitTransactionStatus::Confirmed { block_height } => {
                match block_height {
                    Some(height) => {
                        println!("      (confirmed in block {height}, nothing to broadcast)");
                    }
                    None => println!("      (already confirmed, nothing to broadcast)"),
                }
                continue;
            }
            ExitTransactionStatus::WaitingForDependencies => {
                println!("      (waiting on the transactions it depends on)");
            }
            ExitTransactionStatus::WaitingForTimelock {
                spendable_at_height,
            } => match spendable_at_height {
                Some(height) => println!("      (waiting for its timelock, until block {height})"),
                None => println!("      (waiting for its timelock)"),
            },
            ExitTransactionStatus::Ready | ExitTransactionStatus::Unverified => {}
        }
        let package = match &tx.cpfp_tx_hex {
            Some(cpfp) => format!("{},{}", tx.tx_hex, cpfp),
            None => tx.tx_hex.clone(),
        };
        println!("      Package: {package}");
    }
}

#[cfg(test)]
mod tests {
    use breez_sdk_spark::{UnilateralExitLeaf, UnilateralExitTransaction, UnilateralExitTxKind};

    use super::*;

    /// Covers every status variant, both funding kinds, and each optional field
    /// set and unset, so a field that stops round-tripping is caught here.
    fn sample_exit() -> UnilateralExitResponse {
        UnilateralExitResponse {
            recoverable_value_sat: 100_000,
            total_fee_sat: 1_500,
            cpfp_fee_sat: 900,
            fanout_fee_sat: 400,
            sweep_fee_sat: 200,
            leaves: vec![UnilateralExitLeaf {
                leaf_id: "leaf-1".to_string(),
                value: 100_000,
            }],
            transactions: vec![
                UnilateralExitTransaction {
                    kind: UnilateralExitTxKind::FanOut,
                    node_id: None,
                    txid: "aa".to_string(),
                    tx_hex: "0200".to_string(),
                    cpfp_tx_hex: None,
                    csv_timelock_blocks: None,
                    depends_on: vec![],
                    status: ExitTransactionStatus::Confirmed {
                        block_height: Some(101),
                    },
                },
                UnilateralExitTransaction {
                    kind: UnilateralExitTxKind::Node,
                    node_id: Some("node-1".to_string()),
                    txid: "bb".to_string(),
                    tx_hex: "0201".to_string(),
                    cpfp_tx_hex: Some("0301".to_string()),
                    csv_timelock_blocks: Some(144),
                    depends_on: vec!["aa".to_string()],
                    status: ExitTransactionStatus::Confirmed { block_height: None },
                },
                UnilateralExitTransaction {
                    kind: UnilateralExitTxKind::Refund,
                    node_id: Some("node-1".to_string()),
                    txid: "cc".to_string(),
                    tx_hex: "0202".to_string(),
                    cpfp_tx_hex: Some("0302".to_string()),
                    csv_timelock_blocks: Some(2016),
                    depends_on: vec!["bb".to_string()],
                    status: ExitTransactionStatus::WaitingForTimelock {
                        spendable_at_height: Some(245),
                    },
                },
                UnilateralExitTransaction {
                    kind: UnilateralExitTxKind::Refund,
                    node_id: Some("node-2".to_string()),
                    txid: "dd".to_string(),
                    tx_hex: "0203".to_string(),
                    cpfp_tx_hex: None,
                    csv_timelock_blocks: None,
                    depends_on: vec!["bb".to_string()],
                    status: ExitTransactionStatus::WaitingForDependencies,
                },
                UnilateralExitTransaction {
                    kind: UnilateralExitTxKind::Node,
                    node_id: Some("node-3".to_string()),
                    txid: "ee".to_string(),
                    tx_hex: "0204".to_string(),
                    cpfp_tx_hex: None,
                    csv_timelock_blocks: None,
                    depends_on: vec![],
                    status: ExitTransactionStatus::Unverified,
                },
                UnilateralExitTransaction {
                    kind: UnilateralExitTxKind::Sweep,
                    node_id: None,
                    txid: "ff".to_string(),
                    tx_hex: "0205".to_string(),
                    cpfp_tx_hex: None,
                    csv_timelock_blocks: None,
                    depends_on: vec!["cc".to_string(), "dd".to_string()],
                    status: ExitTransactionStatus::Ready,
                },
            ],
            funding_inputs: vec![
                CpfpInput::P2tr {
                    txid: "11".to_string(),
                    vout: 0,
                    value: 5_000,
                    pubkey: "02ab".to_string(),
                },
                CpfpInput::P2wpkh {
                    txid: "22".to_string(),
                    vout: 3,
                    value: 7_000,
                    pubkey: "03cd".to_string(),
                },
                CpfpInput::Custom {
                    txid: "33".to_string(),
                    vout: 1,
                    value: 9_000,
                    script_pubkey_hex: "5120ef".to_string(),
                    signed_input_weight: 230,
                },
            ],
        }
    }

    /// What `unilateral-exit` writes is what `check-unilateral-exit` reads.
    #[test]
    fn exit_file_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("exit.json");
        let exit = sample_exit();

        write_exit(&path, &exit).expect("write");
        let read_back = read_exit(&path).expect("read");

        // Debug rather than the serialized form: a field that serializes away
        // on both sides would compare equal as JSON while losing its value.
        assert_eq!(format!("{exit:?}"), format!("{read_back:?}"));
    }
}
