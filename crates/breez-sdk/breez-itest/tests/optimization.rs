use anyhow::Result;
use breez_sdk_itest::*;
use rstest::*;

/// Regression test for the missing `WalletEvent::Optimization` →
/// `SdkEvent::Optimization` bridge: the variant existed and was exposed
/// through the bindings, but core dropped the wallet event on the floor,
/// so external listeners never saw any optimization events.
///
/// The fixture disables auto-optimization (so nothing races us) and sets a
/// high multiplicity (so triggering optimization on a freshly-funded
/// single-leaf wallet has real work to do, not a `Skipped` no-op). If the
/// bridge is removed again, this test times out waiting for the terminal
/// event.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_leaf_optimization_emits_events(
    #[future] alice_sdk_manual_opt: Result<SdkInstance>,
) -> Result<()> {
    let mut alice = alice_sdk_manual_opt.await?;
    ensure_funded(&mut alice, 50_000).await?;
    clear_event_receiver(&mut alice.events).await;

    alice.sdk.start_leaf_optimization().await;
    wait_for_optimization_completed_event(&mut alice.events, 180).await?;
    Ok(())
}

/// Server-mode counterpart of [`test_leaf_optimization_emits_events`].
///
/// In server mode (`background_tasks_enabled = false`) the client runtime
/// loop — which used to be the only subscriber of `WalletEvent` — doesn't
/// run, so optimization events would silently disappear if the bridge lived
/// inside that loop. This test asserts the runtime-agnostic forwarder
/// keeps optimization events flowing to external listeners regardless of
/// runtime profile.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_leaf_optimization_emits_events_server_mode(
    #[future] alice_server_sdk_manual_opt: Result<SdkInstance>,
) -> Result<()> {
    let mut alice = alice_server_sdk_manual_opt.await?;
    // Server mode has no ClaimedDeposits event; poll for the balance instead.
    ensure_funded_via_polling(&mut alice, 50_000).await?;
    clear_event_receiver(&mut alice.events).await;

    alice.sdk.start_leaf_optimization().await;
    wait_for_optimization_completed_event(&mut alice.events, 180).await?;
    Ok(())
}
