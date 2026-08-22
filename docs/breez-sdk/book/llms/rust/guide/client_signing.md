# Client signing

Client signing lets a server drive payments while the key that approves them stays with the user. The server prepares the payment and builds a small package that describes it, the user reviews and signs the package on their side, and the server publishes it to complete the payment.

Use it when the SDK runs on your server, for example hosting wallets for many users, and the server must not be able to send payments on its own. It works for Spark addresses and invoices, Lightning invoices, token payments, Bitcoin addresses and LNURL payments.

Client signing is fully opt-in. Without it, `send_payment` works as described in [Sending payments](send_payment.md).

## How it works

1. **Prepare** on the server with `prepare_send_payment`, exactly as in [Sending payments](send_payment.md). This validates the input and returns the fees.
2. **Build** on the server with `build_unsigned_transfer_package`. This returns the one item the user needs to sign. It carries the amount, fee and destination of the payment.
3. **Sign** on the user's side. The user reviews the package and signs it with their signer.
4. **Publish** on the server with `publish_signed_transfer_package` to complete the payment.

Sometimes the wallet first needs to re-shape its funds so it can send the exact amount (a denomination swap). That swap also needs the user's signature, so it arrives as its own package: publishing it returns `PublishSignedTransferPackageResponse::SwapCompleted`, and you build again from the same prepare response. Repeat until publishing returns `PublishSignedTransferPackageResponse::PaymentSent`.

The server keeps no state between these steps. Everything needed to complete the payment travels inside the requests and responses, so building and publishing can happen in different processes or on different instances. This fits [Server mode](server_mode.md) deployments, where an SDK instance is built per request.

## Signing on the user's side

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/signer/trait.ExternalSparkSigner.html

The user's side does not need a connected SDK, only a signer that holds the user's key: any `ExternalSparkSigner` implementation (see [Using an External Signer](external_signer.md)), whether it runs on the user's device or fronts a remote signing service.

The package tells the user exactly what they are approving: the amount, the fee and the destination. Show these to the user before signing. Sign Transfer and Swap packages with `prepare_transfer`, and Token packages with `prepare_token_transaction`:

```rust
let signature = match &unsigned {
    UnsignedTransferPackage::Transfer {
        prepare_transfer,
        amount_sat,
        fee_sat,
        target,
    } => {
        // Show the user what they are approving before signing
        let destination = match target {
            TransferTarget::Spark { address, .. } => address,
            TransferTarget::Lightning { bolt11, .. } => bolt11,
            TransferTarget::CoopExit { address, .. } => address,
        };
        info!("Approve sending {amount_sat} sats (fee {fee_sat} sats) to {destination}");
        TransferSignature::Transfer {
            signed: signer.prepare_transfer(prepare_transfer.clone()).await?,
        }
    }
    UnsignedTransferPackage::Swap {
        prepare_transfer,
        amount_sat,
        fee_sat,
        ..
    } => {
        info!("Approve re-shaping funds for a {amount_sat} sat send (fee {fee_sat} sats)");
        TransferSignature::Transfer {
            signed: signer.prepare_transfer(prepare_transfer.clone()).await?,
        }
    }
    UnsignedTransferPackage::Token {
        prepare_token_transaction,
        token_identifier,
        amount,
        fee,
        is_swap,
        ..
    } => {
        if *is_swap {
            info!("Approve combining token outputs for a {token_identifier} send");
        } else {
            info!("Approve sending {amount} of token {token_identifier} (fee {fee})");
        }
        TransferSignature::Token {
            signed: signer
                .prepare_token_transaction(prepare_token_transaction.clone())
                .await?,
        }
    }
    UnsignedTransferPackage::TokenBatch {
        prepare_token_transaction,
        totals,
        is_swap,
        ..
    } => {
        if *is_swap {
            info!("Approve combining token outputs before the batch is sent");
        } else {
            for total in totals {
                // Unset would mean sats, which a batch cannot send yet
                let token_id = total.token_identifier.as_deref().unwrap_or_default();
                info!("Approve sending {} of token {token_id}", total.amount);
            }
        }
        TransferSignature::Token {
            signed: signer
                .prepare_token_transaction(prepare_token_transaction.clone())
                .await?,
        }
    }
};

let signed_package = SignedTransferPackage {
    unsigned,
    signature,
};
```



## Driving the send from the server

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.build_unsigned_transfer_package

Prepare once, then repeat build, sign and publish until the payment is sent:

```rust
let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: "<spark address or invoice>".to_string(),
        },
        amount: Some(5_000),
        token_identifier: None,
        conversion_options: None,
        fee_policy: None,
    })
    .await?;

loop {
    let unsigned = sdk
        .build_unsigned_transfer_package(BuildUnsignedTransferPackageRequest {
            prepare_response: prepare_response.clone(),
            options: None,
        })
        .await?;

    // Send the package to the user, who reviews and signs it
    let signed_package = sign_package(signer, unsigned).await?;

    match sdk
        .publish_signed_transfer_package(PublishSignedTransferPackageRequest { signed_package })
        .await?
    {
        // The wallet's funds were re-shaped first: build the payment again
        PublishSignedTransferPackageResponse::SwapCompleted => continue,
        PublishSignedTransferPackageResponse::PaymentSent { payment } => {
            return Ok(payment);
        }
        // Only a batch package pays several recipients at once
        PublishSignedTransferPackageResponse::PaymentsSent { .. } => {
            anyhow::bail!("unexpected batch response for a single payment")
        }
    }
}
```



### Bitcoin

For Bitcoin addresses, choose the confirmation speed when building the package. The fee, and therefore what the user signs, depends on it:

```rust
// For Bitcoin address sends, the confirmation speed is chosen when
// building the package: the fee depends on it
let unsigned = sdk
    .build_unsigned_transfer_package(BuildUnsignedTransferPackageRequest {
        prepare_response,
        options: Some(BuildTransferPackageOptions::BitcoinAddress {
            confirmation_speed: OnchainConfirmationSpeed::Medium,
        }),
    })
    .await?;
```



### Lightning

For BOLT11 invoices the build options work like the send options in [Sending payments](send_payment.md#lightning-1): `prefer_spark` sends via a direct Spark transfer when the invoice also contains a Spark address, and `completion_timeout_secs` controls how long publishing waits for the payment to complete before returning it while still pending:

```rust
let unsigned = sdk
    .build_unsigned_transfer_package(BuildUnsignedTransferPackageRequest {
        prepare_response,
        options: Some(BuildTransferPackageOptions::Bolt11Invoice {
            prefer_spark: true,
            completion_timeout_secs: Some(10),
        }),
    })
    .await?;
```



### Tokens

Token payments follow the same loop. Prepare with a token identifier as in [Token payments](token_payments.md). The package amounts are in the token's base units, and the user signs with `prepare_token_transaction`. A Token package with `is_swap` set means the wallet first needs to combine token outputs: publishing it returns `PublishSignedTransferPackageResponse::SwapCompleted`, just like the Bitcoin case.

## LNURL-Pay

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.build_unsigned_lnurl_pay_package

LNURL payments have their own pair of methods, because completing them includes the LNURL exchange with the recipient's service. Prepare with `prepare_lnurl_pay` as in [LNURL-Pay](lnurl_pay.md), then run the same loop with `build_unsigned_lnurl_pay_package` and `publish_signed_lnurl_pay_package`. The result carries the LNURL response, including any success action:

```rust
loop {
    let unsigned = sdk
        .build_unsigned_lnurl_pay_package(BuildUnsignedLnurlPayPackageRequest {
            prepare_response: prepare_response.clone(),
        })
        .await?;

    let signed_package = sign_package(signer, unsigned).await?;

    match sdk
        .publish_signed_lnurl_pay_package(PublishSignedLnurlPayPackageRequest {
            signed_package,
        })
        .await?
    {
        PublishSignedLnurlPayResponse::SwapCompleted => continue,
        PublishSignedLnurlPayResponse::PaymentSent { response } => {
            return Ok(response);
        }
    }
}
```



## Failures and retries

- Publishing the same signed package twice returns the same result, so it is safe to retry after a lost response or a network error.
- If publishing fails because the wallet's funds moved or fees changed since the package was built, prepare again and restart the loop with a fresh package.
- Never reuse a signature for a changed payment. Any change to the amount, fee or destination needs a new package, reviewed and signed by the user.

## Remote signers

The signature does not have to come from a device holding the mnemonic. Any `ExternalSparkSigner` implementation can sign the package, including one backed by a remote signing service. With Turnkey, a policy can require the end user to approve the transfer signing while the server runs the rest; see [Using Turnkey](turnkey.md#user-approved-payments).

## Limitations

- Payments with a conversion step (see [Converting tokens](token_conversion.md)) are not supported.
- USDC/USDT cross-chain sends are not supported.
