use anyhow::Result;
use breez_sdk_itest::*;
use breez_sdk_spark::{
    OptimizationMode, OptimizationOutcome, OptimizeLeavesRequest, SdkError, SdkEvent,
};
use rstest::*;
use tokio::sync::mpsc;
use tracing::info;

/// End-to-end test for `optimize_leaves` in `Full` mode (client runtime).
///
/// The fixture disables auto-optimization (so nothing races the test) and
/// uses a high multiplicity so the planner produces real swaps. We expect
/// `Completed { rounds_executed > 0 }`.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_optimize_leaves_full_client_mode(
    #[future] alice_sdk_manual_opt: Result<SdkInstance>,
) -> Result<()> {
    let mut alice = alice_sdk_manual_opt.await?;
    ensure_funded(&mut alice, 50_000).await?;
    clear_event_receiver(&mut alice.events).await;

    // The default request selects Full mode.
    let outcome = alice
        .sdk
        .optimize_leaves(OptimizeLeavesRequest::default())
        .await?
        .outcome;

    match outcome {
        OptimizationOutcome::Completed { rounds_executed } => {
            assert!(
                rounds_executed > 0,
                "Full mode should execute at least one round on a deliberately under-optimized wallet"
            );
            info!("Full mode completed with {rounds_executed} rounds");
        }
        other => panic!("expected Completed, got {other:?}"),
    }

    assert_no_auto_optimization_events(&mut alice.events).await;
    Ok(())
}

/// End-to-end test for `optimize_leaves` in `SingleRound` mode (client
/// runtime).
///
/// Drives the loop manually: each call should return `InProgress` while
/// work remains, then `Completed` exactly once.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_optimize_leaves_single_round_client_mode(
    #[future] alice_sdk_manual_opt: Result<SdkInstance>,
) -> Result<()> {
    let mut alice = alice_sdk_manual_opt.await?;
    ensure_funded(&mut alice, 50_000).await?;
    clear_event_receiver(&mut alice.events).await;

    let final_rounds = drive_single_round_loop(&alice).await?;
    assert!(
        final_rounds > 0,
        "SingleRound mode should execute at least one round on a deliberately under-optimized wallet"
    );
    info!("SingleRound mode completed after {final_rounds} rounds");

    assert_no_auto_optimization_events(&mut alice.events).await;
    Ok(())
}

/// Server-mode counterpart: in server mode there is no
/// `BackgroundProcessor` running auto-optimization, so the only way leaves
/// get optimized is via the manual API. Asserts the manual path works
/// regardless of runtime profile.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_optimize_leaves_full_server_mode(
    #[future] alice_server_sdk_manual_opt: Result<SdkInstance>,
) -> Result<()> {
    let mut alice = alice_server_sdk_manual_opt.await?;
    // Server mode has no ClaimedDeposits event; poll the balance instead.
    ensure_funded_via_polling(&mut alice, 50_000).await?;
    clear_event_receiver(&mut alice.events).await;

    let outcome = alice
        .sdk
        .optimize_leaves(OptimizeLeavesRequest::default())
        .await?
        .outcome;

    match outcome {
        OptimizationOutcome::Completed { rounds_executed } => {
            assert!(rounds_executed > 0);
            info!("Server-mode Full completed with {rounds_executed} rounds");
        }
        other => panic!("expected Completed, got {other:?}"),
    }

    assert_no_auto_optimization_events(&mut alice.events).await;
    Ok(())
}

/// Concurrent calls must reject with `OptimizationAlreadyRunning`.
///
/// Joins two `Full`-mode calls concurrently: whichever the runtime polls
/// first acquires the `is_running` lock; the other observes the lock held
/// and returns `OptimizationAlreadyRunning`. The test doesn't care which
/// side wins — only that the loser sees the typed error.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_optimize_leaves_rejects_concurrent_calls(
    #[future] alice_sdk_manual_opt: Result<SdkInstance>,
) -> Result<()> {
    let mut alice = alice_sdk_manual_opt.await?;
    ensure_funded(&mut alice, 50_000).await?;
    clear_event_receiver(&mut alice.events).await;

    let (r1, r2) = tokio::join!(
        alice.sdk.optimize_leaves(OptimizeLeavesRequest::default()),
        alice.sdk.optimize_leaves(OptimizeLeavesRequest::default()),
    );

    let ok_count = [&r1, &r2].iter().filter(|r| r.is_ok()).count();
    let rejected_count = [&r1, &r2]
        .iter()
        .filter(|r| matches!(r, Err(SdkError::OptimizationAlreadyRunning)))
        .count();

    assert_eq!(
        ok_count, 1,
        "exactly one concurrent call should succeed (r1={r1:?}, r2={r2:?})"
    );
    assert_eq!(
        rejected_count, 1,
        "exactly one concurrent call should be rejected with AlreadyRunning (r1={r1:?}, r2={r2:?})"
    );

    Ok(())
}

/// Drives a `SingleRound` loop to completion, returning the cumulative
/// round count observed. The SDK signals the final round by returning
/// `Completed` in the same call (when the planner reports a
/// single-swap, fully-converging plan). `Completed { rounds_executed: 0 }`
/// only fires when the call sees an already-optimal wallet.
async fn drive_single_round_loop(alice: &SdkInstance) -> Result<u32> {
    let mut total = 0u32;
    let mut saw_in_progress = false;
    // Safety bound — the wallet is deliberately small.
    for iteration in 0..50 {
        let outcome = alice
            .sdk
            .optimize_leaves(OptimizeLeavesRequest {
                mode: OptimizationMode::SingleRound,
            })
            .await?
            .outcome;
        match outcome {
            OptimizationOutcome::InProgress => {
                total += 1;
                saw_in_progress = true;
                info!("SingleRound iteration {iteration}: round {total} (InProgress)");
            }
            OptimizationOutcome::Completed { rounds_executed: 0 } => {
                assert!(
                    !saw_in_progress,
                    "Completed{{0}} after seeing InProgress — convergence hint should have caused Completed{{1}} instead"
                );
                info!("SingleRound iteration {iteration}: Completed (wallet already optimal)");
                return Ok(0);
            }
            OptimizationOutcome::Completed { rounds_executed } => {
                assert_eq!(
                    rounds_executed, 1,
                    "SingleRound mode should execute at most one round per call"
                );
                total += rounds_executed;
                info!("SingleRound iteration {iteration}: round {total} (Completed)");
                return Ok(total);
            }
        }
    }
    panic!("SingleRound loop did not terminate within 50 iterations");
}

/// Drains the event channel for a brief window and asserts no
/// `AutoOptimization` events were emitted — they're reserved for the auto
/// path, and manual `optimize_leaves` calls must stay silent.
async fn assert_no_auto_optimization_events(events: &mut mpsc::Receiver<SdkEvent>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
    while let Ok(Some(event)) =
        tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), events.recv()).await
    {
        assert!(
            !matches!(event, SdkEvent::AutoOptimization { .. }),
            "manual optimize_leaves should not emit AutoOptimization events, got: {event:?}"
        );
    }
}
