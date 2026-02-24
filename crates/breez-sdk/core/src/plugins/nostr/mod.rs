use crate::{models::PaymentStatus, sdk::SdkServices};

mod error;
mod event;
mod model;

use bitcoin::hashes::{Hash, sha256};
use breez_nostr::{
    error::{NostrError, NostrResult},
    model::LightningInvoice,
    nips::nip47::model::{
        GetBalanceResponse, GetInfoResponse, ListTransactionsRequest, LookupInvoiceResponse,
        MakeInvoiceRequest, MakeInvoiceResponse, PayInvoiceRequest, PayInvoiceResponse,
    },
    sdk_services::SdkEventListener,
};
use breez_sdk_common::input::InputType;
use event::NostrEventListener;
use hex::ToHex;
use spark_wallet::InvoiceDescription;

#[macros::async_trait]
impl breez_nostr::NostrSdkServices for SdkServices {
    fn supported_methods(&self) -> &[&'static str] {
        &["get_info"]
    }

    async fn make_invoice(&self, req: &MakeInvoiceRequest) -> NostrResult<MakeInvoiceResponse> {
        let invoice_res = self
            .spark_wallet
            .create_lightning_invoice(
                req.amount / 1000,
                req.description
                    .as_ref()
                    .map(|d| InvoiceDescription::Memo(d.clone())),
                None,
                req.expiry.and_then(|e| e.try_into().ok()),
                false,
            )
            .await
            .map_err(|e| NostrError::generic(e.to_string()))?;
        let Some(preimage) = invoice_res.payment_preimage else {
            return Err(NostrError::generic("No payment preimage found."));
        };
        let preimage = hex::decode(&preimage)
            .map_err(|err| NostrError::generic(format!("Could not decode preimage: {err}")))?;
        let payment_hash = sha256::Hash::hash(&preimage);
        Ok(MakeInvoiceResponse {
            invoice: invoice_res.invoice,
            payment_hash: payment_hash.encode_hex(),
        })
    }

    async fn pay_invoice(&self, req: &PayInvoiceRequest) -> NostrResult<PayInvoiceResponse> {
        let payment_res = self
            .pay_lightning_invoice(&req.invoice, req.amount.and_then(|a| a.checked_mul(1_000)))
            .await
            .map_err(Into::<NostrError>::into)?;

        let Some(payment) = payment_res.lightning_payment else {
            return Err(NostrError::generic(
                "No lightning payment information associated with the payment",
            ));
        };

        let preimage = payment
            .payment_preimage
            .ok_or_else(|| NostrError::generic("Payment did not return any preimage"))?;

        Ok(PayInvoiceResponse {
            preimage,
            fees_paid: Some(payment.fee_sat),
        })
    }

    async fn list_transactions(
        &self,
        req: &ListTransactionsRequest,
    ) -> NostrResult<Vec<LookupInvoiceResponse>> {
        let req = crate::ListPaymentsRequest {
            type_filter: req.transaction_type.map(|t| vec![t.into()]),
            status_filter: Some(match req.unpaid {
                Some(true) => vec![PaymentStatus::Completed, PaymentStatus::Pending],
                _ => vec![PaymentStatus::Completed],
            }),
            from_timestamp: req.from.map(|t| t.as_u64()),
            to_timestamp: req.until.map(|t| t.as_u64()),
            offset: req.offset.and_then(|o| o.try_into().ok()),
            limit: req.limit.and_then(|l| l.try_into().ok()),
            sort_ascending: None,
            asset_filter: None,
            payment_details_filter: None,
        };

        let mut result = vec![];
        for payment in self
            .list_payments(req)
            .await
            .map_err(Into::<NostrError>::into)?
            .payments
        {
            let Ok(lookup_invoice_res) = payment.try_into() else {
                continue;
            };
            result.push(lookup_invoice_res);
        }

        Ok(result)
    }

    async fn get_balance(&self) -> NostrResult<GetBalanceResponse> {
        self.get_info(crate::GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await
        .map(|res| GetBalanceResponse {
            balance: res.balance_sats.saturating_mul(1_000),
        })
        .map_err(Into::into)
    }

    async fn get_info(&self) -> NostrResult<GetInfoResponse> {
        Ok(GetInfoResponse {
            methods: self
                .supported_methods()
                .iter()
                .map(std::string::ToString::to_string)
                .collect(),
            notifications: vec![],
            alias: None,
            color: None,
            pubkey: None,
            network: None,
            block_height: None,
            block_hash: None,
        })
    }

    async fn add_event_listener(&self, listener: Box<dyn SdkEventListener>) -> String {
        let listener = Box::new(NostrEventListener { inner: listener });
        self.add_event_listener(listener).await
    }

    async fn remove_event_listener(&self, listener_id: String) {
        self.remove_event_listener(&listener_id).await;
    }

    async fn parse_invoice(&self, invoice: &str) -> NostrResult<LightningInvoice> {
        let res = breez_sdk_common::input::parse(invoice, None)
            .await
            .map_err(|err| NostrError::generic(format!("Could not parse invoice: {err}")))?;

        let InputType::Bolt11Invoice(details) = res else {
            return Err(NostrError::generic(
                "Got unexpected input type while parsing bolt11 invoice.",
            ));
        };

        Ok(LightningInvoice {
            bolt11: details.invoice.bolt11,
            payment_hash: details.payment_hash,
            description: details.description,
            amount_msat: details.amount_msat,
        })
    }
}
