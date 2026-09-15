# Client signing

Client signing lets a server drive payments while the key that approves them stays with the user. The server prepares the payment and builds a small package that describes it, the user reviews and signs the package on their side, and the server publishes it to complete the payment.

Use it when the SDK runs on your server, for example hosting wallets for many users, and the server must not be able to send payments on its own. It works for Spark addresses and invoices, Lightning invoices, token payments, Bitcoin addresses and LNURL payments.

Client signing is fully opt-in. Without it, `SendPayment` works as described in [Sending payments](send_payment.md).

## How it works

1. **Prepare** on the server with `PrepareSendPayment`, exactly as in [Sending payments](send_payment.md). This validates the input and returns the fees.
2. **Build** on the server with `BuildUnsignedTransferPackage`. This returns the one item the user needs to sign. It carries the amount, fee and destination of the payment.
3. **Sign** on the user's side. The user reviews the package and signs it with their signer.
4. **Publish** on the server with `PublishSignedTransferPackage` to complete the payment.

Sometimes the wallet first needs to re-shape its funds so it can send the exact amount (a denomination swap). That swap also needs the user's signature, so it arrives as its own package: publishing it returns `PublishSignedTransferPackageResponseSwapCompleted`, and you build again from the same prepare response. Repeat until publishing returns `PublishSignedTransferPackageResponsePaymentSent`.

The server keeps no state between these steps. Everything needed to complete the payment travels inside the requests and responses, so building and publishing can happen in different processes or on different instances. This fits [Server mode](server_mode.md) deployments, where an SDK instance is built per request.

## Signing on the user's side

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/signer/trait.ExternalSparkSigner.html

The user's side does not need a connected SDK, only a signer that holds the user's key: any `ExternalSparkSigner` implementation (see [Using an External Signer](external_signer.md)), whether it runs on the user's device or fronts a remote signing service.

The package tells the user exactly what they are approving: the amount, the fee and the destination. Show these to the user before signing. Sign Transfer and Swap packages with `PrepareTransfer`, and Token packages with `PrepareTokenTransaction`:

```go
var signature breez_sdk_spark.TransferSignature

switch pkg := unsigned.(type) {
case breez_sdk_spark.UnsignedTransferPackageTransfer:
	// Show the user what they are approving before signing
	var destination string
	switch target := pkg.Target.(type) {
	case breez_sdk_spark.TransferTargetSpark:
		destination = target.Address
	case breez_sdk_spark.TransferTargetLightning:
		destination = target.Bolt11
	case breez_sdk_spark.TransferTargetCoopExit:
		destination = target.Address
	}
	log.Printf(
		"Approve sending %v sats (fee %v sats) to %v",
		pkg.AmountSat,
		pkg.FeeSat,
		destination,
	)
	signed, err := signer.PrepareTransfer(pkg.PrepareTransfer)
	if err != nil {
		return breez_sdk_spark.SignedTransferPackage{}, err
	}
	signature = breez_sdk_spark.TransferSignatureTransfer{Signed: signed}
case breez_sdk_spark.UnsignedTransferPackageSwap:
	log.Printf(
		"Approve re-shaping funds for a %v sat send (fee %v sats)",
		pkg.AmountSat,
		pkg.FeeSat,
	)
	signed, err := signer.PrepareTransfer(pkg.PrepareTransfer)
	if err != nil {
		return breez_sdk_spark.SignedTransferPackage{}, err
	}
	signature = breez_sdk_spark.TransferSignatureTransfer{Signed: signed}
case breez_sdk_spark.UnsignedTransferPackageToken:
	if pkg.IsSwap {
		log.Printf(
			"Approve combining token outputs for a %v send",
			pkg.TokenIdentifier,
		)
	} else {
		log.Printf(
			"Approve sending %v of token %v (fee %v)",
			pkg.Amount,
			pkg.TokenIdentifier,
			pkg.Fee,
		)
	}
	signed, err := signer.PrepareTokenTransaction(pkg.PrepareTokenTransaction)
	if err != nil {
		return breez_sdk_spark.SignedTransferPackage{}, err
	}
	signature = breez_sdk_spark.TransferSignatureToken{Signed: signed}
case breez_sdk_spark.UnsignedTransferPackageTokenBatch:
	if pkg.IsSwap {
		log.Printf("Approve combining token outputs before the batch is sent")
	} else {
		for _, total := range pkg.Totals {
			// Unset would mean sats, which a batch cannot send yet
			tokenID := ""
			if total.TokenIdentifier != nil {
				tokenID = *total.TokenIdentifier
			}
			log.Printf(
				"Approve sending %v of token %s",
				total.Amount,
				tokenID,
			)
		}
	}
	signed, err := signer.PrepareTokenTransaction(pkg.PrepareTokenTransaction)
	if err != nil {
		return breez_sdk_spark.SignedTransferPackage{}, err
	}
	signature = breez_sdk_spark.TransferSignatureToken{Signed: signed}
}

signedPackage := breez_sdk_spark.SignedTransferPackage{
	Unsigned:  unsigned,
	Signature: signature,
}
```



## Driving the send from the server

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.build_unsigned_transfer_package

Prepare once, then repeat build, sign and publish until the payment is sent:

```go
paymentRequest := "<spark address or invoice>"
amountSats := new(big.Int).SetInt64(5_000)

prepareResponse, err := sdk.PrepareSendPayment(breez_sdk_spark.PrepareSendPaymentRequest{
	PaymentRequest:    breez_sdk_spark.PaymentRequestInput{Input: paymentRequest},
	Amount:            &amountSats,
	TokenIdentifier:   nil,
	ConversionOptions: nil,
	FeePolicy:         nil,
})
if err != nil {
	return nil, err
}

for {
	unsigned, err := sdk.BuildUnsignedTransferPackage(
		breez_sdk_spark.BuildUnsignedTransferPackageRequest{
			PrepareResponse: prepareResponse,
			Options:         nil,
		},
	)
	if err != nil {
		return nil, err
	}

	// Send the package to the user, who reviews and signs it
	signedPackage, err := SignPackage(signer, unsigned)
	if err != nil {
		return nil, err
	}

	response, err := sdk.PublishSignedTransferPackage(
		breez_sdk_spark.PublishSignedTransferPackageRequest{
			SignedPackage: signedPackage,
		},
	)
	if err != nil {
		return nil, err
	}

	switch result := response.(type) {
	// The wallet's funds were re-shaped first: build the payment again
	case breez_sdk_spark.PublishSignedTransferPackageResponseSwapCompleted:
		continue
	case breez_sdk_spark.PublishSignedTransferPackageResponsePaymentSent:
		return &result.Payment, nil
	// Only a batch package pays several recipients at once
	case breez_sdk_spark.PublishSignedTransferPackageResponsePaymentsSent:
		return nil, errors.New("unexpected batch response for a single payment")
	}
}
```



### Bitcoin

For Bitcoin addresses, choose the confirmation speed when building the package. The fee, and therefore what the user signs, depends on it:

```go
// For Bitcoin address sends, the confirmation speed is chosen when
// building the package: the fee depends on it
var options breez_sdk_spark.BuildTransferPackageOptions
options = breez_sdk_spark.BuildTransferPackageOptionsBitcoinAddress{
	ConfirmationSpeed: breez_sdk_spark.OnchainConfirmationSpeedMedium,
}

unsigned, err := sdk.BuildUnsignedTransferPackage(
	breez_sdk_spark.BuildUnsignedTransferPackageRequest{
		PrepareResponse: prepareResponse,
		Options:         &options,
	},
)
if err != nil {
	return err
}
```



### Lightning

For BOLT11 invoices the build options work like the send options in [Sending payments](send_payment.md#lightning-1): `PreferSpark` sends via a direct Spark transfer when the invoice also contains a Spark address, and `CompletionTimeoutSecs` controls how long publishing waits for the payment to complete before returning it while still pending:

```go
var completionTimeoutSecs uint32 = 10
var options breez_sdk_spark.BuildTransferPackageOptions
options = breez_sdk_spark.BuildTransferPackageOptionsBolt11Invoice{
	PreferSpark:           true,
	CompletionTimeoutSecs: &completionTimeoutSecs,
}

unsigned, err := sdk.BuildUnsignedTransferPackage(
	breez_sdk_spark.BuildUnsignedTransferPackageRequest{
		PrepareResponse: prepareResponse,
		Options:         &options,
	},
)
if err != nil {
	return err
}
```



### Tokens

Token payments follow the same loop. Prepare with a token identifier as in [Token payments](token_payments.md). The package amounts are in the token's base units, and the user signs with `PrepareTokenTransaction`. A Token package with `IsSwap` set means the wallet first needs to combine token outputs: publishing it returns `PublishSignedTransferPackageResponseSwapCompleted`, just like the Bitcoin case.

## LNURL-Pay

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.build_unsigned_lnurl_pay_package

LNURL payments have their own pair of methods, because completing them includes the LNURL exchange with the recipient's service. Prepare with `PrepareLnurlPay` as in [LNURL-Pay](lnurl_pay.md), then run the same loop with `BuildUnsignedLnurlPayPackage` and `PublishSignedLnurlPayPackage`. The result carries the LNURL response, including any success action:

```go
for {
	unsigned, err := sdk.BuildUnsignedLnurlPayPackage(
		breez_sdk_spark.BuildUnsignedLnurlPayPackageRequest{
			PrepareResponse: prepareResponse,
		},
	)
	if err != nil {
		return nil, err
	}

	signedPackage, err := SignPackage(signer, unsigned)
	if err != nil {
		return nil, err
	}

	response, err := sdk.PublishSignedLnurlPayPackage(
		breez_sdk_spark.PublishSignedLnurlPayPackageRequest{
			SignedPackage: signedPackage,
		},
	)
	if err != nil {
		return nil, err
	}

	switch result := response.(type) {
	case breez_sdk_spark.PublishSignedLnurlPayResponseSwapCompleted:
		continue
	case breez_sdk_spark.PublishSignedLnurlPayResponsePaymentSent:
		return &result.Response, nil
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
