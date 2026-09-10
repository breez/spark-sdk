# Conditional Payments

Conditional payments use Hash Time-Locked Contracts (HTLCs) to lock funds with a cryptographic hash of a secret preimage and an expiration time. The payment can only be claimed by revealing the preimage before expiration. If not claimed in time, the funds are automatically returned to the sender. This enables use cases like atomic cross-chain swaps.

The SDK supports both sending conditional payments via Spark HTLCs and receiving them via HODL invoices.

**Developer note**

Preimages are required to be unique and are not managed by the SDK. It is your responsibility as a developer to manage them, including how to generate them, store them, and provide them when claiming payments.

## Sending Spark HTLC payments

HTLC payments use the standard payment API described in [Sending payments](send_payment.md). To create an HTLC payment, prepare the payment normally, then provide the Spark HTLC options when [sending](send_payment.md#spark). These options include the payment hash (SHA-256 hash of the preimage) and the expiry duration.

```rust
let payment_request = "<spark address>".to_string();
// Set the amount you wish to pay the receiver
let amount_sats = Some(50_000);
let prepare_request = PrepareSendPaymentRequest {
    payment_request: PaymentRequest::Input {
        input: payment_request,
    },
    amount: amount_sats,
    token_identifier: None,
    conversion_options: None,
    fee_policy: None,
};
let prepare_response = sdk.prepare_send_payment(prepare_request).await?;

// If the fees are acceptable, continue to create the HTLC Payment
if let SendPaymentMethod::SparkAddress { fee, .. } = prepare_response.payment_method {
    info!("Fees: {} sats", fee);
}

let preimage = "<32-byte unique preimage hex>";
let preimage_bytes = hex::decode(preimage)?;
let payment_hash_bytes = sha256::digest(preimage_bytes);
let payment_hash = hex::encode(payment_hash_bytes);

// Set the HTLC options
let options = SendPaymentOptions::SparkAddress {
    htlc_options: Some(SparkHtlcOptions {
        payment_hash,
        expiry_duration_secs: 1000,
    }),
};

let request = SendPaymentRequest {
    prepare_response,
    options: Some(options),
    idempotency_key: None,
};
let send_response = sdk.send_payment(request).await?;
let payment = send_response.payment;
```



## Receiving using HODL invoices

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

You can receive using HODL invoices — Lightning invoices where the payment is held until you claim it by revealing the preimage. To create one, provide a `payment_hash` when calling `receive_payment` with the `ReceivePaymentMethod::Bolt11Invoice` payment method.

```rust
let preimage = "<32-byte unique preimage hex>";
let preimage_bytes = hex::decode(preimage)?;
let payment_hash_bytes = sha256::digest(preimage_bytes);
let payment_hash = hex::encode(payment_hash_bytes);

let response = sdk
    .receive_payment(ReceivePaymentRequest {
        payment_method: ReceivePaymentMethod::Bolt11Invoice {
            description: "HODL invoice".to_string(),
            amount_sats: Some(50_000),
            expiry_secs: None,
            payment_hash: Some(payment_hash),
            receiver_identity_public_key: None,
        },
    })
    .await?;

let invoice = response.payment_request;
info!("HODL invoice: {invoice}");
```



## Listing claimable conditional payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

Once detected, claimable HTLC payments are immediately listed as pending in the [list of payments](/llms/rust/guide/list_payments.md). Additionally, a `SdkEvent::PaymentPending` event is emitted to notify your application. See [Listening to events](/llms/rust/guide/events.md) for more details.

To list only claimable HTLC payments, you can filter by HTLC status. This works for both Spark HTLC payments and HODL invoices.

```rust
let request = ListPaymentsRequest {
    type_filter: Some(vec![PaymentType::Receive]),
    status_filter: Some(vec![PaymentStatus::Pending]),
    payment_details_filter: Some(vec![
        PaymentDetailsFilter::Spark {
            htlc_status: Some(vec![SparkHtlcStatus::WaitingForPreimage]),
            conversion_refund_needed: None,
        },
        PaymentDetailsFilter::Lightning {
            htlc_status: Some(vec![SparkHtlcStatus::WaitingForPreimage]),
        },
    ]),
    ..Default::default()
};

let response = sdk.list_payments(request).await?;
let payments = response.payments;

for payment in &payments {
    match &payment.details {
        Some(PaymentDetails::Spark {
            htlc_details: Some(htlc),
            ..
        }) => {
            info!("Spark HTLC expiry time: {}", htlc.expiry_time);
        }
        Some(PaymentDetails::Lightning {
            htlc_details: htlc, ..
        }) => {
            info!("Lightning HTLC expiry time: {}", htlc.expiry_time);
        }
        _ => {}
    }
}
```



## Claiming conditional payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.claim_htlc_payment

To claim an HTLC payment, provide the preimage that matches the payment hash. This works for both Spark HTLC payments and HODL invoices.

```rust
let preimage = "<preimage hex>".to_string();
let response = sdk
    .claim_htlc_payment(ClaimHtlcPaymentRequest { preimage })
    .await?;
let payment = response.payment;
```
