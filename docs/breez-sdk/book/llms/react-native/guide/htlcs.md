# Conditional Payments

Conditional payments use Hash Time-Locked Contracts (HTLCs) to lock funds with a cryptographic hash of a secret preimage and an expiration time. The payment can only be claimed by revealing the preimage before expiration. If not claimed in time, the funds are automatically returned to the sender. This enables use cases like atomic cross-chain swaps.

The SDK supports both sending conditional payments via Spark HTLCs and receiving them via HODL invoices.

**Developer note**

Preimages are required to be unique and are not managed by the SDK. It is your responsibility as a developer to manage them, including how to generate them, store them, and provide them when claiming payments.

## Sending Spark HTLC payments

HTLC payments use the standard payment API described in [Sending payments](send_payment.md). To create an HTLC payment, prepare the payment normally, then provide the Spark HTLC options when [sending](send_payment.md#spark). These options include the payment hash (SHA-256 hash of the preimage) and the expiry duration.

```typescript
const paymentRequest = '<spark address>'
// Set the amount you wish to pay the receiver
const amountSats = BigInt(50_000)
const prepareRequest = {
  paymentRequest: new PaymentRequest.Input({ input: paymentRequest }),
  amount: amountSats,
  tokenIdentifier: undefined,
  conversionOptions: undefined,
  feePolicy: undefined
}
const prepareResponse = await sdk.prepareSendPayment(prepareRequest)

// If the fees are acceptable, continue to create the HTLC Payment
if (prepareResponse.paymentMethod?.tag === SendPaymentMethod_Tags.SparkAddress) {
  const fee = prepareResponse.paymentMethod.inner.fee
  console.debug(`Fees: ${fee} sats`)
}

const preimage = '<32-byte unique preimage hex>'
const preimageBuffer = Buffer.from(preimage, 'hex')
const paymentHash = createHash('sha256').update(preimageBuffer).digest('hex')

// Set the HTLC options
const options = new SendPaymentOptions.SparkAddress({
  htlcOptions: {
    paymentHash,
    expiryDurationSecs: BigInt(1000)
  }
})

const request = {
  prepareResponse,
  options,
  idempotencyKey: undefined
}
const sendResponse = await sdk.sendPayment(request)
const payment = sendResponse.payment
```



## Receiving using HODL invoices

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

You can receive using HODL invoices — Lightning invoices where the payment is held until you claim it by revealing the preimage. To create one, provide a `paymentHash` when calling `receivePayment` with the `ReceivePaymentMethod.Bolt11Invoice` payment method.

```typescript
const preimage = '<32-byte unique preimage hex>'
const preimageBuffer = Buffer.from(preimage, 'hex')
const paymentHash = createHash('sha256').update(preimageBuffer).digest('hex')

const response = await sdk.receivePayment({
  paymentMethod: new ReceivePaymentMethod.Bolt11Invoice({
    description: 'HODL invoice',
    amountSats: BigInt(50_000),
    expirySecs: undefined,
    paymentHash
  })
})

const invoice = response.paymentRequest
console.log(`HODL invoice: ${invoice}`)
```



## Listing claimable conditional payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

Once detected, claimable HTLC payments are immediately listed as pending in the [list of payments](/llms/react-native/guide/list_payments.md). Additionally, a `SdkEvent.PaymentPending` event is emitted to notify your application. See [Listening to events](/llms/react-native/guide/events.md) for more details.

To list only claimable HTLC payments, you can filter by HTLC status. This works for both Spark HTLC payments and HODL invoices.

```typescript
const request = {
  typeFilter: [PaymentType.Receive],
  statusFilter: [PaymentStatus.Pending],
  paymentDetailsFilter: [new PaymentDetailsFilter.Spark({
    htlcStatus: [SparkHtlcStatus.WaitingForPreimage],
    conversionRefundNeeded: undefined
  }), new PaymentDetailsFilter.Lightning({
    htlcStatus: [SparkHtlcStatus.WaitingForPreimage]
  })],
  assetFilter: undefined,
  fromTimestamp: undefined,
  toTimestamp: undefined,
  offset: undefined,
  limit: undefined,
  sortAscending: undefined
}

const response = await sdk.listPayments(request)
const payments = response.payments

for (const payment of payments) {
  if (payment.details?.tag === PaymentDetails_Tags.Spark) {
    const htlc = payment.details.inner.htlcDetails
    if (htlc != null) {
      console.log(`Spark HTLC expiry time: ${htlc.expiryTime}`)
    }
  } else if (payment.details?.tag === PaymentDetails_Tags.Lightning) {
    const htlc = payment.details.inner.htlcDetails
    console.log(`Lightning HTLC expiry time: ${htlc.expiryTime}`)
  }
}
```



## Claiming conditional payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.claim_htlc_payment

To claim an HTLC payment, provide the preimage that matches the payment hash. This works for both Spark HTLC payments and HODL invoices.

```typescript
const preimage = '<preimage hex>'
const response = await sdk.claimHtlcPayment(
  { preimage }
)
const payment = response.payment
```
