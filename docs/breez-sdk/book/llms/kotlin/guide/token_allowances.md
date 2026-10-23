# Token allowances

A token allowance lets another wallet pull tokens from your wallet, within limits you set. The wallet that grants the allowance is the owner. The wallet that pulls is the spender.

An allowance names the spender by its Spark address, and sets:

- the token
- a limit on each payment and a limit on all payments together
- an expiry
- an optional list of recipients the spender may pay

The owner can revoke an allowance at any time. An allowance doesn't reserve funds: a pull fails if the owner's balance is too low. In the owner's history, a pull shows up as an ordinary outgoing payment.

Granting and revoking need the device clock to be within about a minute of the network's time.

## Granting an allowance

```kotlin
val response = sdk.createTokenAllowance(
    CreateTokenAllowanceRequest(
        spenderAddress = "<spender spark address>",
        tokenIdentifier = "<token identifier>",
        maxPerPayment = TokenAllowanceLimit.Amount(BigInteger.fromLong(5_000_000L)),
        maxTotal = TokenAllowanceLimit.Amount(BigInteger.fromLong(100_000_000L)),
        expiryTime = 1_798_761_600uL,
        allowedRecipients = emptyList(),
    )
)
// Log.v("Breez", "Allowance id: ${response.allowance.id}")
```



Both limits must be given: to leave one open, set it to `TokenAllowanceLimit.Unlimited`.

Only one allowance can be active for the same spender and token. Granting another returns `TokenAllowanceErrorReason.AlreadyActive`: list your allowances to find the active one. To change the limits, revoke the allowance and grant a new one.

## Listing allowances

Use `TokenAllowanceRole.Owner` for allowances your wallet granted and `TokenAllowanceRole.Spender` for allowances granted to it.

```kotlin
val response = sdk.listTokenAllowances(
    ListTokenAllowancesRequest(
        role = TokenAllowanceRole.OWNER,
        counterpartyAddress = null,
        tokenIdentifier = null,
        includeInactive = false,
        offset = null,
        limit = null,
    )
)
for (allowance in response.allowances) {
    // Log.v("Breez", "${allowance.id}: spent ${allowance.spentAmount}")
}
```



The spent amount counts a pull as soon as it is sent, including pulls that later fail, so it can be higher than what was actually pulled.

## Revoking an allowance

```kotlin
sdk.revokeTokenAllowance(RevokeTokenAllowanceRequest(allowanceId = "<allowance id>"))
```



A revocation takes effect within seconds. A pull that hasn't completed can still fail after a revocation, so treat a pull as paid only once it completes.

## Pulling a payment

A spender pulls in two steps. `preparePullPayment` finds the allowance, checks its limits and builds the transfer from the payer's tokens. `pullPayment` signs it and sends it.

```kotlin
val prepareResponse = sdk.preparePullPayment(
    PreparePullPaymentRequest(
        payerAddress = "<payer spark address>",
        tokenIdentifier = "<token identifier>",
        receivers = listOf(
            PullReceiver(
                amount = BigInteger.fromLong(5_000_000L),
                receiverAddress = null,
            )
        ),
    )
)
// Log.v("Breez", "Pulling ${prepareResponse.amount}")

val response = sdk.pullPayment(PullPaymentRequest(prepareResponse))
// Log.v("Breez", "Pull transaction: ${response.txHash}")
```



Leave a receiver's address unset to pull into your own wallet. One pull can pay several receivers. When the allowance has a recipient list, every receiver must be on it, your own wallet included. The per-payment limit applies to the total.

### Retrying

A pull counts as paid once `pullPayment` returns it with the `status` `PaymentStatus.Completed`. A `PaymentStatus.Pending` pull is accepted but can still fail: call `pullPayment` again with the same prepare response to refresh it.

If `pullPayment` fails with `SdkError.SparkError` or `SdkError.NetworkError`, call it again with the same prepare response; the operators recognize a pull they've already seen, so this never charges the payer twice. Keep calling until the pull completes, fails with `SdkError.TokenAllowance`, or 10 minutes have passed since its `expiryTime`, because a pull the operators accepted can still complete for a few minutes after that time. Prepare a new pull only after that.

`TokenAllowanceErrorReason.PreparedPullStale` means the operators never accepted the pull and never will: the tokens it was prepared with were spent by another payment, or it wasn't sent before its window closed. Prepare a new pull right away. Don't call `pullPayment` twice at the same time with the same prepare response, because the second call can report a pull the first one is still sending as stale.

## Errors

Token allowance errors return `SdkError.TokenAllowance` with a `reason` that says why the request was refused, for example `TokenAllowanceErrorReason.OverPerPaymentLimit`, `TokenAllowanceErrorReason.OverTotalLimit`, `TokenAllowanceErrorReason.RecipientNotAllowed`, `TokenAllowanceErrorReason.Revoked` or `TokenAllowanceErrorReason.PayerInsufficientFunds`. `TokenAllowanceErrorReason.PreparedPullStale` means the pull can't go through anymore (see [Retrying](#retrying)). `TokenAllowanceErrorReason.NotEnabled` means token allowances aren't available on this network.

In JavaScript, errors arrive as text: the reason's name is part of the error message.

## Signing pulls on the client

To have the spender's key approve each pull on a separate device or signing service, see [Client signing](client_signing.md#pulls).
