use std::cmp::Reverse;
use std::str::FromStr;

use bitcoin::hashes::sha256;
use spark_wallet::{Preimage, TransferId, WalletTransfer};

use crate::{
    ClaimHtlcPaymentRequest, ClaimHtlcPaymentResponse, FetchHtlcPaymentsRequest,
    FetchHtlcPaymentsResponse, error::SdkError, models::Payment, sdk::BreezSdk,
};

pub(super) async fn claim_htlc_payment(
    sdk: &BreezSdk,
    request: ClaimHtlcPaymentRequest,
) -> Result<ClaimHtlcPaymentResponse, SdkError> {
    let preimage = Preimage::from_hex(&request.preimage)
        .map_err(|_| SdkError::InvalidInput("Invalid preimage".to_string()))?;
    let transfer_id = request
        .transfer_id
        .as_deref()
        .map(TransferId::from_str)
        .transpose()
        .map_err(|e| SdkError::InvalidInput(format!("Invalid transfer id: {e}")))?;
    let payment_hash = preimage.compute_hash();

    let claimable_htlc_transfers = sdk.spark_wallet.list_claimable_htlc_transfers(None).await?;
    ensure_claimable_htlc(
        &claimable_htlc_transfers,
        payment_hash,
        transfer_id.as_ref(),
    )?;

    let transfer = sdk
        .spark_wallet
        .claim_htlc(&preimage, transfer_id.as_ref())
        .await?;
    let payment: Payment = transfer.try_into()?;

    // Insert the payment into storage to make it immediately available for listing
    sdk.storage.apply_payment_update(payment.clone()).await?;

    Ok(ClaimHtlcPaymentResponse { payment })
}

fn ensure_claimable_htlc(
    claimable: &[WalletTransfer],
    payment_hash: sha256::Hash,
    transfer_id: Option<&TransferId>,
) -> Result<(), SdkError> {
    let transfer = claimable
        .iter()
        .find(|t| {
            t.htlc_preimage_request
                .as_ref()
                .is_some_and(|p| p.payment_hash == payment_hash)
        })
        .ok_or_else(|| {
            SdkError::InvalidInput("No claimable HTLC with the given payment hash".to_string())
        })?;
    match transfer_id {
        Some(transfer_id) if transfer.id != *transfer_id => Err(SdkError::InvalidInput(format!(
            "The claimable HTLC with the given payment hash is {}, not {transfer_id}",
            transfer.id
        ))),
        _ => Ok(()),
    }
}

pub(super) async fn fetch_htlc_payments(
    sdk: &BreezSdk,
    request: FetchHtlcPaymentsRequest,
) -> Result<FetchHtlcPaymentsResponse, SdkError> {
    let payment_hash = sha256::Hash::from_str(&request.payment_hash)
        .map_err(|e| SdkError::InvalidInput(format!("Invalid payment hash: {e}")))?;
    let transfers = sdk
        .spark_wallet
        .list_htlc_transfers(&payment_hash.to_string())
        .await?;
    Ok(FetchHtlcPaymentsResponse {
        payments: htlc_payments(transfers)?,
    })
}

fn htlc_payments(transfers: Vec<WalletTransfer>) -> Result<Vec<Payment>, SdkError> {
    let mut payments = transfers
        .into_iter()
        .map(Payment::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    payments.sort_by_key(|payment| Reverse(payment.timestamp));
    Ok(payments)
}

#[cfg(test)]
mod tests {
    use bitcoin::hashes::Hash;
    use macros::test_all;
    use platform_utils::time::SystemTime;
    use spark_wallet::{
        PreimageRequest, PreimageRequestStatus, PublicKey, Transfer, TransferStatus, TransferType,
    };

    use super::*;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn pk(fill_byte: u8) -> PublicKey {
        let mut bytes = [fill_byte; 33];
        bytes[0] = 2;
        PublicKey::from_slice(&bytes).unwrap()
    }

    fn received_htlc(payment_hash: sha256::Hash, created_time: u64) -> WalletTransfer {
        let (sender, ours, ssp) = (pk(7), pk(1), pk(2));
        WalletTransfer::from_transfer(
            Transfer {
                id: TransferId::generate(),
                sender_identity_public_key: sender,
                receiver_identity_public_key: ours,
                status: TransferStatus::SenderKeyTweakPending,
                total_value: 1_000,
                expiry_time: None,
                leaves: Vec::new(),
                created_time: Some(created_time),
                updated_time: Some(created_time),
                transfer_type: TransferType::PreimageSwap,
                spark_invoice: None,
            },
            None,
            Some(PreimageRequest {
                payment_hash,
                status: PreimageRequestStatus::WaitingForPreimage,
                created_time: SystemTime::now(),
                expiry_time: SystemTime::now(),
                preimage: None,
            }),
            ours,
            ssp,
        )
    }

    #[test_all]
    fn a_claimable_htlc_with_the_payment_hash_can_be_claimed() {
        let payment_hash = sha256::Hash::hash(b"preimage");
        let claimable = [received_htlc(payment_hash, 100)];

        assert!(ensure_claimable_htlc(&claimable, payment_hash, None).is_ok());
    }

    #[test_all]
    fn a_payment_hash_without_a_claimable_htlc_is_rejected() {
        let claimable = [received_htlc(sha256::Hash::hash(b"another preimage"), 100)];

        let result = ensure_claimable_htlc(&claimable, sha256::Hash::hash(b"preimage"), None);

        assert!(
            matches!(result, Err(SdkError::InvalidInput(_))),
            "{result:?}"
        );
    }

    #[test_all]
    fn naming_the_claimable_htlc_is_accepted() {
        let payment_hash = sha256::Hash::hash(b"preimage");
        let claimable = [received_htlc(payment_hash, 100)];

        let result = ensure_claimable_htlc(&claimable, payment_hash, Some(&claimable[0].id));

        assert!(result.is_ok(), "{result:?}");
    }

    #[test_all]
    fn naming_another_transfer_is_rejected() {
        let payment_hash = sha256::Hash::hash(b"preimage");
        let claimable = [received_htlc(payment_hash, 100)];
        let other = TransferId::generate();

        let result = ensure_claimable_htlc(&claimable, payment_hash, Some(&other));

        assert!(
            matches!(result, Err(SdkError::InvalidInput(_))),
            "{result:?}"
        );
    }

    #[test_all]
    fn htlc_payments_are_listed_newest_first() {
        let payment_hash = sha256::Hash::hash(b"preimage");
        let older = received_htlc(payment_hash, 100);
        let newer = received_htlc(payment_hash, 200);
        let expected = [newer.id.to_string(), older.id.to_string()];

        let payments = htlc_payments(vec![older, newer]).unwrap();

        let ids: Vec<String> = payments.into_iter().map(|p| p.id).collect();
        assert_eq!(ids, expected);
    }
}
