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

```go
maxPerPayment := breez_sdk_spark.TokenAllowanceLimitAmount{
	Amount: new(big.Int).SetInt64(5_000_000),
}
maxTotal := breez_sdk_spark.TokenAllowanceLimitAmount{
	Amount: new(big.Int).SetInt64(100_000_000),
}

response, err := sdk.CreateTokenAllowance(breez_sdk_spark.CreateTokenAllowanceRequest{
	SpenderAddress:    "<spender spark address>",
	TokenIdentifier:   "<token identifier>",
	MaxPerPayment:     maxPerPayment,
	MaxTotal:          maxTotal,
	ExpiryTime:        1_798_761_600,
	AllowedRecipients: []string{},
})
if err != nil {
	return err
}
log.Printf("Allowance id: %v", response.Allowance.Id)
```



Both limits must be given: to leave one open, set it to `TokenAllowanceLimitUnlimited`.

Only one allowance can be active for the same spender and token. Granting another returns `TokenAllowanceErrorReasonAlreadyActive`: list your allowances to find the active one. To change the limits, revoke the allowance and grant a new one.

## Listing allowances

Use `TokenAllowanceRoleOwner` for allowances your wallet granted and `TokenAllowanceRoleSpender` for allowances granted to it.

```go
includeInactive := false

response, err := sdk.ListTokenAllowances(breez_sdk_spark.ListTokenAllowancesRequest{
	Role:                breez_sdk_spark.TokenAllowanceRoleOwner,
	CounterpartyAddress: nil,
	TokenIdentifier:     nil,
	IncludeInactive:     &includeInactive,
	Offset:              nil,
	Limit:               nil,
})
if err != nil {
	return err
}
for _, allowance := range response.Allowances {
	log.Printf("%v: spent %v", allowance.Id, allowance.SpentAmount)
}
```



The spent amount counts a pull as soon as it is sent, including pulls that later fail, so it can be higher than what was actually pulled.

## Revoking an allowance

```go
err := sdk.RevokeTokenAllowance(breez_sdk_spark.RevokeTokenAllowanceRequest{
	AllowanceId: "<allowance id>",
})
if err != nil {
	return err
}
```



A revocation takes effect within seconds. A pull that hasn't completed can still fail after a revocation, so treat a pull as paid only once it completes.

## Pulling a payment

A spender pulls in two steps. `PreparePullPayment` finds the allowance, checks its limits and builds the transfer from the payer's tokens. `PullPayment` signs it and sends it.

```go
prepareResponse, err := sdk.PreparePullPayment(breez_sdk_spark.PreparePullPaymentRequest{
	PayerAddress:    "<payer spark address>",
	TokenIdentifier: "<token identifier>",
	Receivers: []breez_sdk_spark.PullReceiver{
		{
			Amount:          new(big.Int).SetInt64(5_000_000),
			ReceiverAddress: nil,
		},
	},
})
if err != nil {
	return err
}
log.Printf("Pulling %v", prepareResponse.Amount)

response, err := sdk.PullPayment(breez_sdk_spark.PullPaymentRequest{
	PrepareResponse: prepareResponse,
})
if err != nil {
	return err
}
log.Printf("Pull transaction: %v", response.TxHash)
```



Leave a receiver's address unset to pull into your own wallet. One pull can pay several receivers. When the allowance has a recipient list, every receiver must be on it, your own wallet included. The per-payment limit applies to the total.

### Retrying

A pull counts as paid once `PullPayment` returns it with the `Status` `PaymentStatusCompleted`. A `PaymentStatusPending` pull is accepted but can still fail: call `PullPayment` again with the same prepare response to refresh it.

If `PullPayment` fails with `SdkErrorSparkError` or `SdkErrorNetworkError`, call it again with the same prepare response; the operators recognize a pull they've already seen, so this never charges the payer twice. Keep calling until the pull completes, fails with `SdkErrorTokenAllowance`, or 10 minutes have passed since its `ExpiryTime`, because a pull the operators accepted can still complete for a few minutes after that time. Prepare a new pull only after that.

`TokenAllowanceErrorReasonPreparedPullStale` means the operators never accepted the pull and never will: the tokens it was prepared with were spent by another payment, or it wasn't sent before its window closed. Prepare a new pull right away. Don't call `PullPayment` twice at the same time with the same prepare response, because the second call can report a pull the first one is still sending as stale.

## Errors

Token allowance errors return `SdkErrorTokenAllowance` with a `Reason` that says why the request was refused, for example `TokenAllowanceErrorReasonOverPerPaymentLimit`, `TokenAllowanceErrorReasonOverTotalLimit`, `TokenAllowanceErrorReasonRecipientNotAllowed`, `TokenAllowanceErrorReasonRevoked` or `TokenAllowanceErrorReasonPayerInsufficientFunds`. `TokenAllowanceErrorReasonPreparedPullStale` means the pull can't go through anymore (see [Retrying](#retrying)). `TokenAllowanceErrorReasonNotEnabled` means token allowances aren't available on this network.

In JavaScript, errors arrive as text: the reason's name is part of the error message.

## Signing pulls on the client

To have the spender's key approve each pull on a separate device or signing service, see [Client signing](client_signing.md#pulls).
