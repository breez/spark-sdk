use std::fs;
use std::path::{Path, PathBuf};

use breez_sdk_spark::signer::single_key_cpfp_signer;
use breez_sdk_spark::{
    BreezSdk, CheckRecoverFundsRequest, CpfpFundingKind, CpfpInput, ExitLeafSelection,
    ExitTransactionStatus, ImportUnilateralExitStateRequest, PrepareRecoverFundsRequest,
    PrepareRecoverFundsResponse, RecoverFundsRequest, RecoverFundsResponse, RecoveryMethod,
    RecoveryVerdict, SkippedLeaf, SkippedLeafReason,
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
    /// Recover the funds that left the balance, or with `--all` every leaf. Quotes
    /// the recovery (which leaves, how each is recovered, the fees, how much to
    /// fund), asks for funding UTXOs and their key when a unilateral exit needs
    /// them, and signs it once you confirm. A cooperative recovery needs the
    /// operators online.
    RecoverFunds {
        /// Target fee rate in sat/vByte.
        #[arg(long)]
        fee_rate: u64,
        /// Funding UTXO kind.
        #[arg(long, value_enum, default_value_t = FundingKindArg::P2tr)]
        funding_kind: FundingKindArg,
        /// Destination address for the recovered funds.
        #[arg(long)]
        destination: String,
        /// Recover every leaf worth it, including the ones still in the balance.
        /// Only for when the operators are unreachable or refuse to serve the
        /// wallet.
        #[arg(long, conflicts_with = "leaf_ids")]
        all: bool,
        /// Leaf id to recover (repeatable). Omit to recover the leaves that left
        /// the balance.
        #[arg(long = "leaf")]
        leaf_ids: Vec<String>,
        /// File to write the signed recovery to, for `check-recover-funds` to read
        /// back.
        #[arg(long)]
        output_file: Option<PathBuf>,
    },
    /// Read a recovery written by `recover-funds` back against the chain: which
    /// of its transactions confirmed, which are ready to broadcast now, and
    /// whether it can still finish.
    CheckRecoverFunds {
        /// File the recovery was written to.
        #[arg(long)]
        input_file: PathBuf,
        /// File to write the updated recovery to. Defaults to `--input-file`.
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
        AdvancedCommand::RecoverFunds {
            fee_rate,
            funding_kind,
            destination,
            all,
            leaf_ids,
            output_file,
        } => {
            let request = PrepareRecoverFundsRequest {
                fee_rate_sat_per_vbyte: fee_rate,
                funding_kind: Some(funding_kind.into()),
                destination,
                selection: recovery_selection(all, leaf_ids),
            };
            recover_funds(rl, sdk, request, funding_kind, output_file.as_deref()).await
        }
        AdvancedCommand::CheckRecoverFunds {
            input_file,
            output_file,
        } => check_recover_funds(sdk, &input_file, output_file.as_deref()).await,
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

async fn recover_funds(
    rl: &mut Editor<CliHelper, DefaultHistory>,
    sdk: &BreezSdk,
    request: PrepareRecoverFundsRequest,
    funding_kind: FundingKindArg,
    output_file: Option<&Path>,
) -> Result<bool, anyhow::Error> {
    let mut prepared = sdk.prepare_recover_funds(request.clone()).await?;
    if prepared.leaves.is_empty() {
        println!("Nothing to recover.");
        print_skipped(&prepared.skipped);
        return Ok(true);
    }
    print_quote(&prepared)?;
    if output_file.is_none() {
        println!(
            "Without --output-file the recovery is only printed: check-recover-funds cannot read \
             it back."
        );
    }

    let mut funding_inputs = Vec::new();
    let mut signer = None;
    if let Some(single_utxo_sats) = prepared.funding.as_ref().map(|f| f.single_utxo_sats) {
        let utxo_line = rl.readline(&format!(
            "Funding UTXO(s) of at least {single_utxo_sats} sats, as txid:vout:value:pubkey \
             (space-separated; for P2TR the internal key; blank to skip the unilateral exit): ",
        ))?;
        if utxo_line.trim().is_empty() {
            let cooperative: Vec<String> = prepared
                .leaves
                .iter()
                .filter(|leaf| leaf.method == RecoveryMethod::Cooperative)
                .map(|leaf| leaf.leaf_id.clone())
                .collect();
            if cooperative.is_empty() {
                println!("Nothing to recover without funding.");
                return Ok(true);
            }
            println!("Recovering only the cooperative leaves:");
            prepared = sdk
                .prepare_recover_funds(PrepareRecoverFundsRequest {
                    selection: ExitLeafSelection::Specific {
                        leaf_ids: cooperative,
                    },
                    ..request
                })
                .await?;
            print_quote(&prepared)?;
        } else {
            funding_inputs = utxo_line
                .split_whitespace()
                .map(|u| parse_cpfp_input(u, funding_kind))
                .collect::<Result<Vec<_>, _>>()?;
            let key_line = rl.readline("Hex secret key for the funding UTXO(s): ")?;
            signer = Some(single_key_cpfp_signer(hex::decode(key_line.trim())?)?);
        }
    }

    let answer = rl.readline_with_initial("Sign this recovery? (y/n): ", ("y", ""))?;
    if !answer.trim().eq_ignore_ascii_case("y") {
        return Ok(true);
    }
    let response = sdk
        .recover_funds(
            RecoverFundsRequest {
                prepared,
                funding_inputs,
            },
            signer,
        )
        .await?;
    print_recovery(&response);
    match output_file {
        Some(output_file) => {
            write_recovery(output_file, &response)?;
            println!(
                "Next: broadcast the Ready packages. After new blocks, run check-recover-funds \
                 --input-file {} to see what is ready next.",
                output_file.display()
            );
        }
        None => println!("Next: broadcast the Ready packages."),
    }
    Ok(true)
}

fn print_quote(prepared: &PrepareRecoverFundsResponse) -> Result<(), anyhow::Error> {
    print_value(prepared)?;
    let cooperative = prepared
        .leaves
        .iter()
        .filter(|leaf| leaf.method == RecoveryMethod::Cooperative)
        .count();
    println!(
        "{} leaf(s), {cooperative} cooperative and {} unilateral: recovering {} sats for {} sats \
         in fees",
        prepared.leaves.len(),
        prepared.leaves.len().saturating_sub(cooperative),
        prepared.recoverable_value_sats,
        prepared.total_fee_sats,
    );
    print_skipped(&prepared.skipped);
    Ok(())
}

fn print_skipped(skipped: &[SkippedLeaf]) {
    for leaf in skipped {
        let reason = match &leaf.reason {
            SkippedLeafReason::FeeExceedsValue => "recovering it costs too much at this fee rate",
            SkippedLeafReason::FundsNotFound => "its funds were not found on-chain",
            SkippedLeafReason::NotRecoverable { message } => message,
        };
        println!(
            "Left out: leaf {} ({} sats): {reason}",
            leaf.leaf_id, leaf.value_sats
        );
    }
}

async fn check_recover_funds(
    sdk: &BreezSdk,
    input_file: &Path,
    output_file: Option<&Path>,
) -> Result<bool, anyhow::Error> {
    let recovery = read_recovery(input_file)?;
    let checked = sdk
        .check_recover_funds(CheckRecoverFundsRequest { recovery })
        .await?;
    println!("Verdict: {:?}", checked.verdict);
    if let RecoveryVerdict::Redo { .. } = checked.verdict {
        println!(
            "  (this recovery cannot finish: run {})",
            redo_command(&checked.recovery)
        );
    }
    print_recovery(&checked.recovery);
    write_recovery(output_file.unwrap_or(input_file), &checked.recovery)?;
    Ok(true)
}

fn redo_command(recovery: &RecoverFundsResponse) -> String {
    let mut command = format!(
        "recover-funds --fee-rate {} --destination {}",
        recovery.fee_rate_sat_per_vbyte, recovery.destination
    );
    for leaf in &recovery.leaves {
        command.push_str(" --leaf ");
        command.push_str(&leaf.leaf_id);
    }
    command
}

fn read_recovery(path: &Path) -> Result<RecoverFundsResponse, anyhow::Error> {
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

/// Writes through a temporary file, so an interrupted write leaves the previous
/// recovery intact.
fn write_recovery(path: &Path, recovery: &RecoverFundsResponse) -> Result<(), anyhow::Error> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    fs::write(&temporary, serde_json::to_string_pretty(recovery)?)?;
    fs::rename(&temporary, path)?;
    println!("Wrote the recovery to {}", path.display());
    Ok(())
}

/// `--all`, the named leaves, or else the leaves that left the balance.
fn recovery_selection(all: bool, leaf_ids: Vec<String>) -> ExitLeafSelection {
    if all {
        ExitLeafSelection::All
    } else if leaf_ids.is_empty() {
        ExitLeafSelection::RecoverableOnly
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
    let vout = vout
        .parse::<u32>()
        .map_err(|e| anyhow::anyhow!("invalid vout in funding UTXO '{s}': {e}"))?;
    let value = value
        .parse::<u64>()
        .map_err(|e| anyhow::anyhow!("invalid value in funding UTXO '{s}': {e}"))?;
    let pubkey = pubkey.to_string();
    Ok(match kind {
        FundingKindArg::P2wpkh => CpfpInput::P2wpkh {
            txid,
            vout,
            value_sats: value,
            pubkey,
        },
        FundingKindArg::P2tr => CpfpInput::P2tr {
            txid,
            vout,
            value_sats: value,
            pubkey,
        },
    })
}

/// Prints each recovery transaction with a copy-pasteable `Package:` line: the
/// tx hex, plus its signed CPFP child when present, comma-separated. That is the
/// form mempool.space accepts for a package broadcast. Confirmed steps need no
/// broadcast, so they show no package.
fn print_recovery(response: &RecoverFundsResponse) {
    println!(
        "Recoverable {} sats, total fee {} sats (cooperative {}, cpfp {}, fanout {}, \
         sweep {}), {} transaction(s):",
        response.recoverable_value_sats,
        response.total_fee_sats,
        response.cooperative_fee_sats,
        response.cpfp_fee_sats,
        response.fanout_fee_sats,
        response.sweep_fee_sats,
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
        let node = tx
            .node_id
            .as_ref()
            .map(|id| format!(" node={id}"))
            .unwrap_or_default();
        println!(
            "  [{i}] {:?}{node} status={:?} txid={}{after}{csv}",
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
    if !response.failed.is_empty() {
        println!("Not recovered, {} leaf(s):", response.failed.len());
    }
    for failure in &response.failed {
        println!(
            "  leaf {} (output {}:{}): {}",
            failure.leaf_id, failure.output_txid, failure.output_vout, failure.error,
        );
    }
}

#[cfg(test)]
mod tests {
    use breez_sdk_spark::{
        CooperativeRecoveryError, CooperativeRecoveryFailure, RecoverFundsLeaf, RecoveryMethod,
        RecoveryTransaction, RecoveryTxKind,
    };

    use super::*;

    fn transaction(
        kind: RecoveryTxKind,
        node_id: Option<&str>,
        txid: &str,
        cpfp: Option<&str>,
        status: ExitTransactionStatus,
    ) -> RecoveryTransaction {
        RecoveryTransaction {
            kind,
            node_id: node_id.map(ToString::to_string),
            txid: txid.to_string(),
            tx_hex: format!("02{txid}"),
            cpfp_tx_hex: cpfp.map(ToString::to_string),
            csv_timelock_blocks: None,
            depends_on: vec![],
            status,
        }
    }

    fn failure(leaf_id: &str, error: CooperativeRecoveryError) -> CooperativeRecoveryFailure {
        CooperativeRecoveryFailure {
            leaf_id: leaf_id.to_string(),
            output_txid: "99".to_string(),
            output_vout: 1,
            error,
        }
    }

    /// One of every kind and status, and each optional field set and unset.
    fn sample_transactions() -> Vec<RecoveryTransaction> {
        let mut refund = transaction(
            RecoveryTxKind::Refund,
            Some("leaf-2"),
            "dd",
            Some("0302"),
            ExitTransactionStatus::WaitingForTimelock {
                spendable_at_height: Some(245),
            },
        );
        refund.csv_timelock_blocks = Some(2016);
        refund.depends_on = vec!["cc".to_string()];
        let mut sweep = transaction(
            RecoveryTxKind::Sweep,
            None,
            "ee",
            None,
            ExitTransactionStatus::WaitingForDependencies,
        );
        sweep.depends_on = vec!["dd".to_string()];
        vec![
            transaction(
                RecoveryTxKind::Cooperative,
                Some("leaf-1"),
                "aa",
                None,
                ExitTransactionStatus::Ready,
            ),
            transaction(
                RecoveryTxKind::FanOut,
                None,
                "bb",
                None,
                ExitTransactionStatus::Confirmed {
                    block_height: Some(240),
                },
            ),
            transaction(
                RecoveryTxKind::Node,
                Some("node-1"),
                "cc",
                Some("0301"),
                ExitTransactionStatus::Confirmed { block_height: None },
            ),
            refund,
            sweep,
            transaction(
                RecoveryTxKind::Node,
                Some("node-2"),
                "ff",
                Some("0303"),
                ExitTransactionStatus::Unverified,
            ),
            transaction(
                RecoveryTxKind::Refund,
                Some("leaf-3"),
                "gg",
                Some("0304"),
                ExitTransactionStatus::WaitingForTimelock {
                    spendable_at_height: None,
                },
            ),
        ]
    }

    /// Covers both recovery methods, every failure and funding input kind, and
    /// the transactions of [`sample_transactions`], so a field that stops
    /// round-tripping is caught here.
    fn sample_recovery() -> RecoverFundsResponse {
        RecoverFundsResponse {
            recoverable_value_sats: 150_000,
            total_fee_sats: 1_800,
            cooperative_fee_sats: 300,
            cpfp_fee_sats: 900,
            fanout_fee_sats: 400,
            sweep_fee_sats: 200,
            leaves: vec![
                RecoverFundsLeaf {
                    leaf_id: "leaf-1".to_string(),
                    value_sats: 50_000,
                    method: RecoveryMethod::Cooperative,
                },
                RecoverFundsLeaf {
                    leaf_id: "leaf-2".to_string(),
                    value_sats: 100_000,
                    method: RecoveryMethod::Unilateral,
                },
            ],
            failed: vec![
                failure(
                    "leaf-3",
                    CooperativeRecoveryError::ReplacementFeeTooLow {
                        required_fee_sats: 500,
                        required_fee_rate_sat_per_vbyte: 4,
                    },
                ),
                failure(
                    "leaf-4",
                    CooperativeRecoveryError::OperatorsUnavailable {
                        message: "unreachable".to_string(),
                    },
                ),
                failure(
                    "leaf-5",
                    CooperativeRecoveryError::Generic {
                        message: "refused".to_string(),
                    },
                ),
            ],
            transactions: sample_transactions(),
            funding_inputs: vec![
                CpfpInput::P2tr {
                    txid: "11".to_string(),
                    vout: 0,
                    value_sats: 5_000,
                    pubkey: "02ab".to_string(),
                },
                CpfpInput::P2wpkh {
                    txid: "12".to_string(),
                    vout: 1,
                    value_sats: 6_000,
                    pubkey: "03cd".to_string(),
                },
                CpfpInput::Custom {
                    txid: "13".to_string(),
                    vout: 2,
                    value_sats: 7_000,
                    script_pubkey_hex: "0020ff".to_string(),
                    signed_input_weight: 272,
                },
            ],
            fee_rate_sat_per_vbyte: 2,
            destination: "bcrt1q...".to_string(),
        }
    }

    /// What `recover-funds` writes is what `check-recover-funds` reads.
    #[test]
    fn recovery_file_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("recovery.json");
        let recovery = sample_recovery();

        write_recovery(&path, &recovery).expect("write");
        let read_back = read_recovery(&path).expect("read");

        assert_eq!(format!("{recovery:?}"), format!("{read_back:?}"));
    }

    #[test]
    fn the_flags_pick_the_selection() {
        assert!(matches!(
            recovery_selection(true, Vec::new()),
            ExitLeafSelection::All
        ));
        assert!(matches!(
            recovery_selection(false, Vec::new()),
            ExitLeafSelection::RecoverableOnly
        ));
        assert!(matches!(
            recovery_selection(false, vec!["leaf-1".to_string()]),
            ExitLeafSelection::Specific { leaf_ids } if leaf_ids == ["leaf-1"]
        ));
    }

    #[test]
    fn a_funding_utxo_parses_per_kind() {
        assert!(matches!(
            parse_cpfp_input("11:0:5000:02ab", FundingKindArg::P2tr).unwrap(),
            CpfpInput::P2tr {
                txid,
                vout: 0,
                value_sats: 5_000,
                pubkey,
            } if txid == "11" && pubkey == "02ab"
        ));
        assert!(matches!(
            parse_cpfp_input("12:1:6000:03cd", FundingKindArg::P2wpkh).unwrap(),
            CpfpInput::P2wpkh {
                txid,
                vout: 1,
                value_sats: 6_000,
                pubkey,
            } if txid == "12" && pubkey == "03cd"
        ));
        assert!(parse_cpfp_input("11:x:6000:03cd", FundingKindArg::P2tr).is_err());
        assert!(parse_cpfp_input("11:0:6000", FundingKindArg::P2tr).is_err());
    }
}
