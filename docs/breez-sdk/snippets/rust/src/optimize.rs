use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

async fn run_full_optimization(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: optimize-leaves-full
    let outcome = sdk.optimize_leaves(None).await?;

    match outcome {
        OptimizationOutcome::Completed { rounds_executed } => {
            if rounds_executed == 0 {
                info!("Optimization skipped — wallet already optimal");
            } else {
                info!("Optimization completed in {} rounds", rounds_executed);
            }
        }
        OptimizationOutcome::InProgress => {
            // Full mode runs to completion in one call, so InProgress is
            // not reachable here.
            unreachable!("Full mode never returns InProgress");
        }
    }
    // ANCHOR_END: optimize-leaves-full
    Ok(())
}

async fn run_optimization_one_round_at_a_time(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: optimize-leaves-single-round
    let mut rounds_executed = 0u32;
    loop {
        let options = OptimizeLeavesOptions {
            mode: OptimizationMode::SingleRound,
        };
        match sdk.optimize_leaves(Some(options)).await? {
            OptimizationOutcome::InProgress => {
                rounds_executed += 1;
                info!("Executed round {}", rounds_executed);
            }
            OptimizationOutcome::Completed {
                rounds_executed: this_round,
            } => {
                rounds_executed += this_round;
                if rounds_executed == 0 {
                    info!("Optimization skipped — wallet already optimal");
                } else {
                    info!("Optimization done after {} rounds", rounds_executed);
                }
                break;
            }
        }
    }
    // ANCHOR_END: optimize-leaves-single-round
    Ok(())
}

fn handle_auto_optimization_event(event: AutoOptimizationEvent) {
    // ANCHOR: auto-optimization-events
    match event {
        AutoOptimizationEvent::Started { total_rounds } => {
            info!("Auto-optimization started with {} rounds", total_rounds);
        }
        AutoOptimizationEvent::RoundCompleted {
            current_round,
            total_rounds,
        } => {
            info!(
                "Auto-optimization round {} of {} completed",
                current_round, total_rounds
            );
        }
        AutoOptimizationEvent::Completed => {
            info!("Auto-optimization completed successfully");
        }
        AutoOptimizationEvent::Cancelled => {
            info!("Auto-optimization was cancelled");
        }
        AutoOptimizationEvent::Failed { error } => {
            info!("Auto-optimization failed: {}", error);
        }
        AutoOptimizationEvent::Skipped => {
            info!("Auto-optimization was skipped because leaves are already optimal");
        }
    }
    // ANCHOR_END: auto-optimization-events
}
