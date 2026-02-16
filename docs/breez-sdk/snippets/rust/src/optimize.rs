use anyhow::Result;
use breez_sdk_spark::*;
use log::info;

fn start_optimization(client: &BreezClient) {
    // ANCHOR: start-optimization
    client.optimization().start();
    // ANCHOR_END: start-optimization
}

async fn cancel_optimization(client: &BreezClient) -> Result<()> {
    // ANCHOR: cancel-optimization
    client.optimization().cancel().await?;
    // ANCHOR_END: cancel-optimization
    Ok(())
}

fn get_optimization_progress(client: &BreezClient) {
    // ANCHOR: get-optimization-progress
    let progress = client.optimization().progress();

    info!("Optimization is running: {}", progress.is_running);
    info!("Current round: {}", progress.current_round);
    info!("Total rounds: {}", progress.total_rounds);
    // ANCHOR_END: get-optimization-progress
}

fn leaf_optimization_events(event: LeafOptimizationEvent) {
    // ANCHOR: optimization-events
    match event {
        LeafOptimizationEvent::Started { total_rounds } => {
            info!("Optimization started with {} rounds", total_rounds);
        }
        LeafOptimizationEvent::RoundCompleted {
            current_round,
            total_rounds,
        } => {
            info!(
                "Optimization round {} of {} completed",
                current_round, total_rounds
            );
        }
        LeafOptimizationEvent::Completed => {
            info!("Optimization completed successfully");
        }
        LeafOptimizationEvent::Cancelled => {
            info!("Optimization was cancelled");
        }
        LeafOptimizationEvent::Failed { error } => {
            info!("Optimization failed: {}", error);
        }
        LeafOptimizationEvent::Skipped => {
            info!("Optimization was skipped because leaves are already optimal");
        }
    }
    // ANCHOR_END: optimization-events
}
