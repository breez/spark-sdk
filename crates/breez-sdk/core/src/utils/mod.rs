pub(crate) mod deposit_chain_syncer;
pub(crate) mod expiring_cell;
pub(crate) mod send_payment_validation;
pub(crate) mod token;
pub(crate) mod utxo_fetcher;

/// Runs a future until completion or until a shutdown signal is received.
/// When shutdown is received, logs the exit message and returns.
pub(crate) async fn run_with_shutdown<F, T>(
    mut shutdown: tokio::sync::watch::Receiver<()>,
    exit_message: &str,
    future: F,
) -> Option<T>
where
    F: std::future::Future<Output = T>,
{
    tokio::select! {
        t = future => {
          Some(t)
        }
        _ = shutdown.changed() => {
            tracing::info!("{exit_message}");
            None
        }
    }
}
