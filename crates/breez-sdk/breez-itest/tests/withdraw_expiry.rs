//! What becomes of a withdrawal whose payout the SSP was never asked to
//! broadcast.
//!
//! The operators carve cooperative exits out of their expired-transfer
//! cancellation, so the leaves are unlocked only by the SSP giving up on the
//! exit itself. This pins that behaviour down, and pins down that nothing
//! reports such a withdrawal as paid out in the meantime.

use anyhow::{Result, bail};
use breez_sdk_itest::fixtures::ssp_fault::SspFaultProxy;
use breez_sdk_itest::*;
use breez_sdk_spark::*;
use rand::RngCore;
use rstest::*;
use tracing::info;

const WITHDRAW_SATS: u64 = 15_000;
const FUNDING_SATS: u64 = 50_000;

/// The SSP mutation that asks for the payout to be broadcast. Losing just this
/// one is the whole point: the operators have already committed the leaves to
/// the SSP by the time it is sent, so the funds have left the wallet with
/// nothing scheduled to pay them out.
const COMPLETE_COOP_EXIT_OPERATION: &str = "CompleteCoopExit";

fn regtest_ssp_base_url() -> String {
    default_config(Network::Regtest)
        .spark_config
        .expect("default_config populates spark_config")
        .ssp_config
        .base_url
}

/// Builds the sender pointed at the fault proxy rather than the real SSP.
///
/// Server mode, so nothing syncs on its own: the balance readings here have to
/// reflect an explicit [`BreezSdk::sync_wallet`] rather than whatever a
/// background sync happened to pick up.
async fn build_sender(
    storage_dir: &str,
    seed: [u8; 32],
    backend: BackendChoice,
    ssp_base_url: &str,
) -> Result<SdkInstance> {
    let mut config = default_server_config(Network::Regtest);
    config.api_key = None;
    config.lnurl_domain = None;
    config
        .spark_config
        .as_mut()
        .expect("default_config populates spark_config")
        .ssp_config
        .base_url = ssp_base_url.to_string();
    build_sdk_with_custom_config_and_backend(
        storage_dir.to_string(),
        seed,
        config,
        None,
        false,
        Some(backend),
    )
    .await
}

async fn balance_of(sdk: &BreezSdk) -> Result<u64> {
    sdk.sync_wallet(SyncWalletRequest {}).await?;
    Ok(sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats)
}

/// Keyed on the method: a withdrawal the SSP was never asked to complete comes
/// back with no exit request attached, so it has no `Withdraw` details either.
async fn withdrawals(sdk: &BreezSdk) -> Result<Vec<Payment>> {
    Ok(sdk
        .list_payments(ListPaymentsRequest::default())
        .await?
        .payments
        .into_iter()
        .filter(|payment| matches!(payment.method, PaymentMethod::Withdraw))
        .collect())
}

/// A bitcoin address on `receiver` that a withdrawal can pay out to.
async fn deposit_address(receiver: &BreezSdk) -> Result<String> {
    Ok(receiver
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress { new_address: None },
        })
        .await?
        .payment_request)
}

async fn prepare_withdrawal(
    sender: &BreezSdk,
    address: &str,
) -> Result<PrepareSendPaymentResponse> {
    Ok(sender
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: address.to_string(),
            },
            amount: Some(u128::from(WITHDRAW_SATS)),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?)
}

/// A funded sender behind the fault proxy, a receiver to be paid, and the
/// proxy itself.
struct Fixture {
    receiver: SdkInstance,
    sender: SdkInstance,
    ssp: SspFaultProxy,
    /// Where the withdrawal pays out, on the receiver.
    address: String,
    /// Held for the run: dropping it deletes the sender's storage.
    _sender_dir: tempfile::TempDir,
}

impl Fixture {
    /// `name` distinguishes this test's temp directories from other suites'.
    async fn setup(name: &str) -> Result<Self> {
        let receiver_dir = tempfile::Builder::new()
            .prefix(&format!("breez-sdk-withdraw-{name}-receiver"))
            .tempdir()?;
        let sender_dir = tempfile::Builder::new()
            .prefix(&format!("breez-sdk-withdraw-{name}-sender"))
            .tempdir()?;
        let sender_path = sender_dir.path().to_string_lossy().to_string();

        let receiver = build_sdk_with_dir(
            receiver_dir.path().to_string_lossy().to_string(),
            random_seed(),
            Some(receiver_dir),
        )
        .await?;
        let backend = resolve_backend_choice().await?;
        let ssp =
            SspFaultProxy::start(&regtest_ssp_base_url(), COMPLETE_COOP_EXIT_OPERATION).await?;

        let sender_seed = random_seed();
        let mut sender = build_sender(&sender_path, sender_seed, backend, &ssp.base_url()).await?;
        // Polling rather than the event: server mode runs no background claimer.
        ensure_funded_via_polling(&mut sender, FUNDING_SATS).await?;

        let address = deposit_address(&receiver.sdk).await?;
        info!("Receiver deposit address: {address}");
        Ok(Self {
            receiver,
            sender,
            ssp,
            address,
            _sender_dir: sender_dir,
        })
    }

    async fn disconnect(self) -> Result<()> {
        self.sender.sdk.disconnect().await?;
        self.receiver.sdk.disconnect().await?;
        Ok(())
    }
}

fn random_seed() -> [u8; 32] {
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    seed
}

/// What happens to a withdrawal this client can never finish: the provider
/// gives up on the exit, and the funds have to come back.
///
/// The proxy keeps failing for the whole test, so the completion request never
/// lands and only the provider giving up can put the funds back.
///
/// Proving the drop first is what makes the return mean anything: without it
/// the test would pass on a withdrawal that never committed at all.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_an_expired_withdrawal_returns_its_funds() -> Result<()> {
    let fx = Fixture::setup("expiry").await?;
    let prepare = prepare_withdrawal(&fx.sender.sdk, &fx.address).await?;
    let funded_balance = balance_of(&fx.sender.sdk).await?;
    info!("Sender balance before the withdrawal: {funded_balance}");

    // Strand the withdrawal: committed to the provider, never asked to pay out.
    fx.ssp.start_failing();
    let stranded = fx
        .sender
        .sdk
        .send_payment(SendPaymentRequest {
            prepare_response: prepare,
            options: None,
            idempotency_key: None,
        })
        .await;
    assert!(
        stranded.is_err(),
        "the withdrawal must fail while its completion request is lost, got {stranded:?}"
    );

    // The funds left the wallet: that is what makes the return below meaningful.
    let committed_balance = wait_for(
        || async {
            let balance = balance_of(&fx.sender.sdk).await?;
            if balance < funded_balance {
                return Ok(balance);
            }
            bail!("sender still holds {balance}, the withdrawal has not committed yet");
        },
        120,
    )
    .await?;
    info!("Sender balance while the withdrawal is committed: {committed_balance}");

    // The completion request stays unreachable, so nothing can pay this out.
    // Only the provider giving up on the exit puts the funds back.
    info!("Waiting for the provider to expire the stranded exit and return the funds");
    let returned = wait_for(
        || async {
            let balance = balance_of(&fx.sender.sdk).await?;
            if balance >= funded_balance {
                return Ok(balance);
            }
            let withdrawals = withdrawals(&fx.sender.sdk).await?;
            bail!("sender still at {balance}, want {funded_balance}; withdrawals: {withdrawals:?}");
        },
        420,
    )
    .await?;
    info!("Sender balance after the exit expired: {returned}");

    let settled = withdrawals(&fx.sender.sdk).await?;
    info!("Withdrawals at the end: {settled:?}");
    assert!(
        settled.iter().all(|p| p.status != PaymentStatus::Completed),
        "a withdrawal that expired without paying out was reported completed: {settled:?}"
    );

    fx.disconnect().await
}
