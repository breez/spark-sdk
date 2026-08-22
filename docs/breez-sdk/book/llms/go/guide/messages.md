# Signing and verifying messages

Through signing and verifying messages we can provide proof that a digital signature was created by a private key.

## Signing a message

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.sign_message

By signing a message using the SDK we can provide a digital signature. Anyone with the `message`, `pubkey` and `signature` can verify the signature was created by the private key of this pubkey.

```go
message := "<message to sign>"
// Set to true to get a compact signature rather than a DER
compact := true

signMessageRequest := breez_sdk_spark.SignMessageRequest{
	Message: message,
	Compact: compact,
}
signMessageResponse, err := sdk.SignMessage(signMessageRequest)
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

signature := signMessageResponse.Signature
pubkey := signMessageResponse.Pubkey

log.Printf("Pubkey: %v", pubkey)
log.Printf("Signature: %v", signature)
```



## Verifying a message

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.check_message

You can prove control of a private key by verifying a `message` with it's `signature` and `pubkey`.

```go
message := "<message>"
pubkey := "<pubkey of signer>"
signature := "<message signature>"

checkMessageRequest := breez_sdk_spark.CheckMessageRequest{
	Message:   message,
	Pubkey:    pubkey,
	Signature: signature,
}
checkMessageResponse, err := sdk.CheckMessage(checkMessageRequest)
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

isValid := checkMessageResponse.IsValid

log.Printf("Signature valid: %v", isValid)
```
