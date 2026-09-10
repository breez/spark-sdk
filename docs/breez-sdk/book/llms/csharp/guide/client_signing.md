# Client signing

Client signing lets a server drive payments while the key that approves them stays with the user. The server prepares the payment and builds a small package that describes it, the user reviews and signs the package on their side, and the server publishes it to complete the payment.

Use it when the SDK runs on your server, for example hosting wallets for many users, and the server must not be able to send payments on its own. It works for Spark addresses and invoices, Lightning invoices, token payments, Bitcoin addresses and LNURL payments.

Client signing is fully opt-in. Without it, `SendPayment` works as described in [Sending payments](send_payment.md).

## How it works

1. **Prepare** on the server with `PrepareSendPayment`, exactly as in [Sending payments](send_payment.md). This validates the input and returns the fees.
2. **Build** on the server with `BuildUnsignedTransferPackage`. This returns the one item the user needs to sign. It carries the amount, fee and destination of the payment.
3. **Sign** on the user's side. The user reviews the package and signs it with their signer.
4. **Publish** on the server with `PublishSignedTransferPackage` to complete the payment.

Sometimes the wallet first needs to re-shape its funds so it can send the exact amount (a denomination swap). That swap also needs the user's signature, so it arrives as its own package: publishing it returns `PublishSignedTransferPackageResponse.SwapCompleted`, and you build again from the same prepare response. Repeat until publishing returns `PublishSignedTransferPackageResponse.PaymentSent`.

The server keeps no state between these steps. Everything needed to complete the payment travels inside the requests and responses, so building and publishing can happen in different processes or on different instances. This fits [Server mode](server_mode.md) deployments, where an SDK instance is built per request.

## Signing on the user's side

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/signer/trait.ExternalSparkSigner.html

The user's side does not need a connected SDK, only a signer that holds the user's key: any `ExternalSparkSigner` implementation (see [Using an External Signer](external_signer.md)), whether it runs on the user's device or fronts a remote signing service.

The package tells the user exactly what they are approving: the amount, the fee and the destination. Show these to the user before signing. Sign Transfer and Swap packages with `PrepareTransfer`, and Token packages with `PrepareTokenTransaction`:

```csharp
TransferSignature signature;
switch (unsigned)
{
    case UnsignedTransferPackage.Transfer transfer:
        // Show the user what they are approving before signing
        var destination = transfer.target switch
        {
            TransferTarget.Spark spark => spark.address,
            TransferTarget.Lightning lightning => lightning.bolt11,
            TransferTarget.CoopExit coopExit => coopExit.address,
            _ => throw new Exception("Unknown transfer target")
        };
        Console.WriteLine($"Approve sending {transfer.amountSat} sats " +
            $"(fee {transfer.feeSat} sats) to {destination}");
        signature = new TransferSignature.Transfer(
            signed: await signer.PrepareTransfer(transfer.prepareTransfer)
        );
        break;
    case UnsignedTransferPackage.Swap swap:
        Console.WriteLine("Approve re-shaping funds for a " +
            $"{swap.amountSat} sat send (fee {swap.feeSat} sats)");
        signature = new TransferSignature.Transfer(
            signed: await signer.PrepareTransfer(swap.prepareTransfer)
        );
        break;
    case UnsignedTransferPackage.Token token:
        if (token.isSwap)
        {
            Console.WriteLine("Approve combining token outputs for a " +
                $"{token.tokenIdentifier} send");
        }
        else
        {
            Console.WriteLine($"Approve sending {token.amount} of token " +
                $"{token.tokenIdentifier} (fee {token.fee})");
        }
        signature = new TransferSignature.Token(
            signed: await signer.PrepareTokenTransaction(token.prepareTokenTransaction)
        );
        break;
    case UnsignedTransferPackage.TokenBatch tokenBatch:
        if (tokenBatch.isSwap)
        {
            Console.WriteLine("Approve combining token outputs " +
                "before the batch is sent");
        }
        else
        {
            foreach (var total in tokenBatch.totals)
            {
                Console.WriteLine($"Approve sending {total.amount} of token " +
                    $"{total.tokenIdentifier}");
            }
        }
        signature = new TransferSignature.Token(
            signed: await signer.PrepareTokenTransaction(
                tokenBatch.prepareTokenTransaction)
        );
        break;
    default:
        throw new Exception("Unknown transfer package");
}

var signedPackage = new SignedTransferPackage(unsigned: unsigned, signature: signature);
```



## Driving the send from the server

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.build_unsigned_transfer_package

Prepare once, then repeat build, sign and publish until the payment is sent:

```csharp
var prepareResponse = await sdk.PrepareSendPayment(
    request: new PrepareSendPaymentRequest(
        paymentRequest: new PaymentRequest.Input(input: "<spark address or invoice>"),
        amount: 5_000UL,
        tokenIdentifier: null,
        conversionOptions: null,
        feePolicy: null
    )
);

while (true)
{
    var unsigned = await sdk.BuildUnsignedTransferPackage(
        request: new BuildUnsignedTransferPackageRequest(
            prepareResponse: prepareResponse,
            options: null
        )
    );

    // Send the package to the user, who reviews and signs it
    var signedPackage = await SignPackage(signer, unsigned);

    var response = await sdk.PublishSignedTransferPackage(
        request: new PublishSignedTransferPackageRequest(signedPackage: signedPackage)
    );

    switch (response)
    {
        // The wallet's funds were re-shaped first: build the payment again
        case PublishSignedTransferPackageResponse.SwapCompleted:
            continue;
        case PublishSignedTransferPackageResponse.PaymentSent paymentSent:
            return paymentSent.payment;
        // Only a batch package pays several recipients at once
        case PublishSignedTransferPackageResponse.PaymentsSent:
            throw new Exception("unexpected batch response for a single payment");
    }
}
```



### Bitcoin

For Bitcoin addresses, choose the confirmation speed when building the package. The fee, and therefore what the user signs, depends on it:

```csharp
// For Bitcoin address sends, the confirmation speed is chosen when
// building the package: the fee depends on it
var unsigned = await sdk.BuildUnsignedTransferPackage(
    request: new BuildUnsignedTransferPackageRequest(
        prepareResponse: prepareResponse,
        options: new BuildTransferPackageOptions.BitcoinAddress(
            confirmationSpeed: OnchainConfirmationSpeed.Medium
        )
    )
);
```



### Lightning

For BOLT11 invoices the build options work like the send options in [Sending payments](send_payment.md#lightning-1): `PreferSpark` sends via a direct Spark transfer when the invoice also contains a Spark address, and `CompletionTimeoutSecs` controls how long publishing waits for the payment to complete before returning it while still pending:

```csharp
var unsigned = await sdk.BuildUnsignedTransferPackage(
    request: new BuildUnsignedTransferPackageRequest(
        prepareResponse: prepareResponse,
        options: new BuildTransferPackageOptions.Bolt11Invoice(
            preferSpark: true,
            completionTimeoutSecs: 10
        )
    )
);
```



### Tokens

Token payments follow the same loop. Prepare with a token identifier as in [Token payments](token_payments.md). The package amounts are in the token's base units, and the user signs with `PrepareTokenTransaction`. A Token package with `IsSwap` set means the wallet first needs to combine token outputs: publishing it returns `PublishSignedTransferPackageResponse.SwapCompleted`, just like the Bitcoin case.

## LNURL-Pay

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.build_unsigned_lnurl_pay_package

LNURL payments have their own pair of methods, because completing them includes the LNURL exchange with the recipient's service. Prepare with `PrepareLnurlPay` as in [LNURL-Pay](lnurl_pay.md), then run the same loop with `BuildUnsignedLnurlPayPackage` and `PublishSignedLnurlPayPackage`. The result carries the LNURL response, including any success action:

```csharp
while (true)
{
    var unsigned = await sdk.BuildUnsignedLnurlPayPackage(
        request: new BuildUnsignedLnurlPayPackageRequest(
            prepareResponse: prepareResponse
        )
    );

    var signedPackage = await SignPackage(signer, unsigned);

    var response = await sdk.PublishSignedLnurlPayPackage(
        request: new PublishSignedLnurlPayPackageRequest(signedPackage: signedPackage)
    );

    switch (response)
    {
        case PublishSignedLnurlPayResponse.SwapCompleted:
            continue;
        case PublishSignedLnurlPayResponse.PaymentSent paymentSent:
            return paymentSent.response;
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
