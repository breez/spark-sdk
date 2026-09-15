# Client signing

Client signing lets a server drive payments while the key that approves them stays with the user. The server prepares the payment and builds a small package that describes it, the user reviews and signs the package on their side, and the server publishes it to complete the payment.

Use it when the SDK runs on your server, for example hosting wallets for many users, and the server must not be able to send payments on its own. It works for Spark addresses and invoices, Lightning invoices, token payments, Bitcoin addresses and LNURL payments.

Client signing is fully opt-in. Without it, `send_payment` works as described in [Sending payments](send_payment.md).

## How it works

1. **Prepare** on the server with `prepare_send_payment`, exactly as in [Sending payments](send_payment.md). This validates the input and returns the fees.
2. **Build** on the server with `build_unsigned_transfer_package`. This returns the one item the user needs to sign. It carries the amount, fee and destination of the payment.
3. **Sign** on the user's side. The user reviews the package and signs it with their signer.
4. **Publish** on the server with `publish_signed_transfer_package` to complete the payment.

Sometimes the wallet first needs to re-shape its funds so it can send the exact amount (a denomination swap). That swap also needs the user's signature, so it arrives as its own package: publishing it returns `PublishSignedTransferPackageResponse.SWAP_COMPLETED`, and you build again from the same prepare response. Repeat until publishing returns `PublishSignedTransferPackageResponse.PAYMENT_SENT`.

The server keeps no state between these steps. Everything needed to complete the payment travels inside the requests and responses, so building and publishing can happen in different processes or on different instances. This fits [Server mode](server_mode.md) deployments, where an SDK instance is built per request.

## Signing on the user's side

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/signer/trait.ExternalSparkSigner.html

The user's side does not need a connected SDK, only a signer that holds the user's key: any `ExternalSparkSigner` implementation (see [Using an External Signer](external_signer.md)), whether it runs on the user's device or fronts a remote signing service.

The package tells the user exactly what they are approving: the amount, the fee and the destination. Show these to the user before signing. Sign Transfer and Swap packages with `prepare_transfer`, and Token packages with `prepare_token_transaction`:

```python
if isinstance(unsigned, UnsignedTransferPackage.TRANSFER):
    # Show the user what they are approving before signing
    target = unsigned.target
    destination = ""
    if isinstance(target, TransferTarget.SPARK):
        destination = target.address
    elif isinstance(target, TransferTarget.LIGHTNING):
        destination = target.bolt11
    elif isinstance(target, TransferTarget.COOP_EXIT):
        destination = target.address
    logging.debug(
        f"Approve sending {unsigned.amount_sat} sats"
        f" (fee {unsigned.fee_sat} sats) to {destination}"
    )
    signature = TransferSignature.TRANSFER(
        signed=await signer.prepare_transfer(unsigned.prepare_transfer)
    )
elif isinstance(unsigned, UnsignedTransferPackage.SWAP):
    logging.debug(
        f"Approve re-shaping funds for a {unsigned.amount_sat} sat send"
        f" (fee {unsigned.fee_sat} sats)"
    )
    signature = TransferSignature.TRANSFER(
        signed=await signer.prepare_transfer(unsigned.prepare_transfer)
    )
elif isinstance(unsigned, UnsignedTransferPackage.TOKEN):
    if unsigned.is_swap:
        logging.debug(
            f"Approve combining token outputs for a {unsigned.token_identifier} send"
        )
    else:
        logging.debug(
            f"Approve sending {unsigned.amount} of token"
            f" {unsigned.token_identifier} (fee {unsigned.fee})"
        )
    signature = TransferSignature.TOKEN(
        signed=await signer.prepare_token_transaction(
            unsigned.prepare_token_transaction
        )
    )
elif isinstance(unsigned, UnsignedTransferPackage.TOKEN_BATCH):
    if unsigned.is_swap:
        logging.debug("Approve combining token outputs before the batch is sent")
    else:
        for total in unsigned.totals:
            logging.debug(
                f"Approve sending {total.amount} of token {total.token_identifier}"
            )
    signature = TransferSignature.TOKEN(
        signed=await signer.prepare_token_transaction(
            unsigned.prepare_token_transaction
        )
    )
else:
    raise ValueError("Unknown transfer package variant")

signed_package = SignedTransferPackage(unsigned=unsigned, signature=signature)
```



## Driving the send from the server

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.build_unsigned_transfer_package

Prepare once, then repeat build, sign and publish until the payment is sent:

```python
try:
    prepare_response = await sdk.prepare_send_payment(
        PrepareSendPaymentRequest(
            payment_request=PaymentRequest.INPUT(input="<spark address or invoice>"),
            amount=5_000,
            token_identifier=None,
            conversion_options=None,
            fee_policy=None,
        )
    )

    while True:
        unsigned = await sdk.build_unsigned_transfer_package(
            BuildUnsignedTransferPackageRequest(
                prepare_response=prepare_response, options=None
            )
        )

        # Send the package to the user, who reviews and signs it
        signed_package = await sign_package(signer, unsigned)

        result = await sdk.publish_signed_transfer_package(
            PublishSignedTransferPackageRequest(signed_package=signed_package)
        )
        if isinstance(result, PublishSignedTransferPackageResponse.SWAP_COMPLETED):
            # The wallet's funds were re-shaped first: build the payment again
            continue
        if isinstance(result, PublishSignedTransferPackageResponse.PAYMENT_SENT):
            return result.payment
        # Only a batch package pays several recipients at once
        if isinstance(result, PublishSignedTransferPackageResponse.PAYMENTS_SENT):
            raise ValueError("unexpected batch response for a single payment")
except Exception as error:
    logging.error(error)
    raise
```



### Bitcoin

For Bitcoin addresses, choose the confirmation speed when building the package. The fee, and therefore what the user signs, depends on it:

```python
# For Bitcoin address sends, the confirmation speed is chosen when
# building the package: the fee depends on it
try:
    unsigned = await sdk.build_unsigned_transfer_package(
        BuildUnsignedTransferPackageRequest(
            prepare_response=prepare_response,
            options=BuildTransferPackageOptions.BITCOIN_ADDRESS(
                confirmation_speed=OnchainConfirmationSpeed.MEDIUM
            ),
        )
    )
except Exception as error:
    logging.error(error)
    raise
```



### Lightning

For BOLT11 invoices the build options work like the send options in [Sending payments](send_payment.md#lightning-1): `prefer_spark` sends via a direct Spark transfer when the invoice also contains a Spark address, and `completion_timeout_secs` controls how long publishing waits for the payment to complete before returning it while still pending:

```python
try:
    unsigned = await sdk.build_unsigned_transfer_package(
        BuildUnsignedTransferPackageRequest(
            prepare_response=prepare_response,
            options=BuildTransferPackageOptions.BOLT11_INVOICE(
                prefer_spark=True, completion_timeout_secs=10
            ),
        )
    )
except Exception as error:
    logging.error(error)
    raise
```



### Tokens

Token payments follow the same loop. Prepare with a token identifier as in [Token payments](token_payments.md). The package amounts are in the token's base units, and the user signs with `prepare_token_transaction`. A Token package with `is_swap` set means the wallet first needs to combine token outputs: publishing it returns `PublishSignedTransferPackageResponse.SWAP_COMPLETED`, just like the Bitcoin case.

## LNURL-Pay

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.build_unsigned_lnurl_pay_package

LNURL payments have their own pair of methods, because completing them includes the LNURL exchange with the recipient's service. Prepare with `prepare_lnurl_pay` as in [LNURL-Pay](lnurl_pay.md), then run the same loop with `build_unsigned_lnurl_pay_package` and `publish_signed_lnurl_pay_package`. The result carries the LNURL response, including any success action:

```python
try:
    while True:
        unsigned = await sdk.build_unsigned_lnurl_pay_package(
            BuildUnsignedLnurlPayPackageRequest(prepare_response=prepare_response)
        )

        signed_package = await sign_package(signer, unsigned)

        result = await sdk.publish_signed_lnurl_pay_package(
            PublishSignedLnurlPayPackageRequest(signed_package=signed_package)
        )
        if isinstance(result, PublishSignedLnurlPayResponse.SWAP_COMPLETED):
            continue
        if isinstance(result, PublishSignedLnurlPayResponse.PAYMENT_SENT):
            return result.response
except Exception as error:
    logging.error(error)
    raise
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
