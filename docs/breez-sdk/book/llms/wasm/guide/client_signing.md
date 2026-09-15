# Client signing

Client signing lets a server drive payments while the key that approves them stays with the user. The server prepares the payment and builds a small package that describes it, the user reviews and signs the package on their side, and the server publishes it to complete the payment.

Use it when the SDK runs on your server, for example hosting wallets for many users, and the server must not be able to send payments on its own. It works for Spark addresses and invoices, Lightning invoices, token payments, Bitcoin addresses and LNURL payments.

Client signing is fully opt-in. Without it, `sendPayment` works as described in [Sending payments](send_payment.md).

## How it works

1. **Prepare** on the server with `prepareSendPayment`, exactly as in [Sending payments](send_payment.md). This validates the input and returns the fees.
2. **Build** on the server with `buildUnsignedTransferPackage`. This returns the one item the user needs to sign. It carries the amount, fee and destination of the payment.
3. **Sign** on the user's side. The user reviews the package and signs it with their signer.
4. **Publish** on the server with `publishSignedTransferPackage` to complete the payment.

Sometimes the wallet first needs to re-shape its funds so it can send the exact amount (a denomination swap). That swap also needs the user's signature, so it arrives as its own package: publishing it returns `PublishSignedTransferPackageResponse.SwapCompleted`, and you build again from the same prepare response. Repeat until publishing returns `PublishSignedTransferPackageResponse.PaymentSent`.

The server keeps no state between these steps. Everything needed to complete the payment travels inside the requests and responses, so building and publishing can happen in different processes or on different instances. This fits [Server mode](server_mode.md) deployments, where an SDK instance is built per request.

## Signing on the user's side

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/signer/trait.ExternalSparkSigner.html

The user's side does not need a connected SDK, only a signer that holds the user's key: any `ExternalSparkSigner` implementation (see [Using an External Signer](external_signer.md)), whether it runs on the user's device or fronts a remote signing service.

The package tells the user exactly what they are approving: the amount, the fee and the destination. Show these to the user before signing. Sign Transfer and Swap packages with `prepareTransfer`, and Token packages with `prepareTokenTransaction`:

```typescript
let signature: TransferSignature
switch (unsigned.type) {
  case 'transfer': {
    const { prepareTransfer, amountSat, feeSat, target } = unsigned
    // Show the user what they are approving before signing
    const destination = target.type === 'lightning' ? target.bolt11 : target.address
    console.log(`Approve sending ${amountSat} sats (fee ${feeSat} sats) to ${destination}`)
    signature = {
      type: 'transfer',
      signed: await signer.prepareTransfer(prepareTransfer)
    }
    break
  }
  case 'swap': {
    const { prepareTransfer, amountSat, feeSat } = unsigned
    console.log(`Approve re-shaping funds for a ${amountSat} sat send (fee ${feeSat} sats)`)
    signature = {
      type: 'transfer',
      signed: await signer.prepareTransfer(prepareTransfer)
    }
    break
  }
  case 'token': {
    const { prepareTokenTransaction, tokenIdentifier, amount, fee, isSwap } = unsigned
    if (isSwap) {
      console.log(`Approve combining token outputs for a ${tokenIdentifier} send`)
    } else {
      console.log(`Approve sending ${amount} of token ${tokenIdentifier} (fee ${fee})`)
    }
    signature = {
      type: 'token',
      signed: await signer.prepareTokenTransaction(prepareTokenTransaction)
    }
    break
  }
  case 'tokenBatch': {
    const { prepareTokenTransaction, totals, isSwap } = unsigned
    if (isSwap) {
      console.log('Approve combining token outputs before the batch is sent')
    } else {
      for (const total of totals) {
        console.log(`Approve sending ${total.amount} of token ${total.tokenIdentifier}`)
      }
    }
    signature = {
      type: 'token',
      signed: await signer.prepareTokenTransaction(prepareTokenTransaction)
    }
    break
  }
}

const signedPackage = { unsigned, signature }
```



## Driving the send from the server

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.build_unsigned_transfer_package

Prepare once, then repeat build, sign and publish until the payment is sent:

```typescript
const prepareResponse = await sdk.prepareSendPayment({
  paymentRequest: { type: 'input', input: '<spark address or invoice>' },
  amount: BigInt(5_000),
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: undefined
})

while (true) {
  const unsigned = await sdk.buildUnsignedTransferPackage({
    prepareResponse,
    options: undefined
  })

  // Send the package to the user, who reviews and signs it
  const signedPackage = await signPackage(signer, unsigned)

  const publishResponse = await sdk.publishSignedTransferPackage({ signedPackage })

  if (publishResponse.type === 'swapCompleted') {
    // The wallet's funds were re-shaped first: build the payment again
    continue
  }
  // Only a batch package pays several recipients at once
  if (publishResponse.type === 'paymentsSent') {
    throw new Error('unexpected batch response for a single payment')
  }
  return publishResponse.payment
}
```



### Bitcoin

For Bitcoin addresses, choose the confirmation speed when building the package. The fee, and therefore what the user signs, depends on it:

```typescript
// For Bitcoin address sends, the confirmation speed is chosen when
// building the package: the fee depends on it
const unsigned = await sdk.buildUnsignedTransferPackage({
  prepareResponse,
  options: {
    type: 'bitcoinAddress',
    confirmationSpeed: 'medium'
  }
})
```



### Lightning

For BOLT11 invoices the build options work like the send options in [Sending payments](send_payment.md#lightning-1): `preferSpark` sends via a direct Spark transfer when the invoice also contains a Spark address, and `completionTimeoutSecs` controls how long publishing waits for the payment to complete before returning it while still pending:

```typescript
const unsigned = await sdk.buildUnsignedTransferPackage({
  prepareResponse,
  options: {
    type: 'bolt11Invoice',
    preferSpark: true,
    completionTimeoutSecs: 10
  }
})
```



### Tokens

Token payments follow the same loop. Prepare with a token identifier as in [Token payments](token_payments.md). The package amounts are in the token's base units, and the user signs with `prepareTokenTransaction`. A Token package with `isSwap` set means the wallet first needs to combine token outputs: publishing it returns `PublishSignedTransferPackageResponse.SwapCompleted`, just like the Bitcoin case.

## LNURL-Pay

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.build_unsigned_lnurl_pay_package

LNURL payments have their own pair of methods, because completing them includes the LNURL exchange with the recipient's service. Prepare with `prepareLnurlPay` as in [LNURL-Pay](lnurl_pay.md), then run the same loop with `buildUnsignedLnurlPayPackage` and `publishSignedLnurlPayPackage`. The result carries the LNURL response, including any success action:

```typescript
while (true) {
  const unsigned = await sdk.buildUnsignedLnurlPayPackage({ prepareResponse })

  const signedPackage = await signPackage(signer, unsigned)

  const publishResponse = await sdk.publishSignedLnurlPayPackage({ signedPackage })

  if (publishResponse.type === 'swapCompleted') {
    continue
  }
  return publishResponse.response
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
