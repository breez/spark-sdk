use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

fn start_optimization(sdk: &BreezSdk) {
    // ANCHOR: start-optimization
    sdk.start_leaf_optimization();
    // ANCHOR_END: start-optimization
}

async fn cancel_optimization(sdk: &BreezSdk) -> Result<()> {
    // ANCHOR: cancel-optimization
    sdk.cancel_leaf_optimization().await?;
    // ANCHOR_END: cancel-optimization
    Ok(())
}

fn get_optimization_progress(sdk: &BreezSdk) {
    // ANCHOR: get-optimization-progress
    let progress = sdk.get_leaf_optimization_progress();

    info!("Optimization is running: {}", progress.is_running);
    info!("Current round: {}", progress.current_round);
    info!("Total rounds: {}", progress.total_rounds);
    // ANCHOR_END: get-optimization-progress
}

fn optimization_events(event: OptimizationEvent) {
    // ANCHOR: optimization-events
    match event {
        OptimizationEvent::Started { total_rounds } => {
            info!("Optimization started with {} rounds", total_rounds);
        }
        OptimizationEvent::RoundCompleted {
            current_round,
            total_rounds,
        } => {
            info!(
                "Optimization round {} of {} completed",
                current_round, total_rounds
            );
        }
        OptimizationEvent::Completed => {
            info!("Optimization completed successfully");
        }
        OptimizationEvent::Cancelled => {
            info!("Optimization was cancelled");
        }
        OptimizationEvent::Failed { error } => {
            info!("Optimization failed: {}", error);
        }
        OptimizationEvent::Skipped => {
            info!("Optimization was skipped because leaves are already optimal");
        }
    }
    // ANCHOR_END: optimization-events
}
