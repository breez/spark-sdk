## Issuing tokens

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_token_issuer

The Breez SDK provides a specialized Token Issuer interface for managing custom token issuance on the Spark network using the using the [BTKN protocol](https://docs.spark.money/learn/tokens/hello-btkn). This functionality enables token creators to issue, manage, and control their own tokens with advanced features.

```go
tokenIssuer := sdk.GetTokenIssuer()
```



## Token creation

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.create_issuer_token

Create a custom token with configurable parameters. Define the decimal precision, max supply and if the token can be frozen.

```go
request := breez_sdk_spark.CreateIssuerTokenRequest{
	Name:        "My Token",
	Ticker:      "MTK",
	Decimals:    6,
	IsFreezable: false,
	MaxSupply:   new(big.Int).SetInt64(1_000_000),
}
tokenMetadata, err := tokenIssuer.CreateIssuerToken(request)
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}
log.Printf("Token identifier: %v", tokenMetadata.Identifier)
```



### Creating multiple tokens

Token creation is limited to one token per issuer wallet. If you need to create and then manage more than one token using the same mnemonic, we recommend using different account numbers when initializing the SDK.

```go
accountNumber := uint32(21)

mnemonic := "<mnemonic words>"
var seed breez_sdk_spark.Seed = breez_sdk_spark.SeedMnemonic{
	Mnemonic:   mnemonic,
	Passphrase: nil,
}
apiKey := "<breez api key>"
config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
config.ApiKey = &apiKey
builder := breez_sdk_spark.NewSdkBuilder(config, seed)
builder.WithDefaultStorage("./.data")

// Set the account number for the SDK
builder.WithAccountNumber(accountNumber)

sdk, err := builder.Build()
```



## Supply Management

### Minting a token

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.mint_issuer_token

Mint to increase the circulating supply of the token.

```go
request := breez_sdk_spark.MintIssuerTokenRequest{
	Amount: new(big.Int).SetInt64(1_000),
}
payment, err := tokenIssuer.MintIssuerToken(request)
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}
```



### Burning a token

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.burn_issuer_token

Permanently remove tokens from the circulating supply by burning them.

```go
request := breez_sdk_spark.BurnIssuerTokenRequest{
	Amount: new(big.Int).SetInt64(1_000),
}
payment, err := tokenIssuer.BurnIssuerToken(request)
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}
```



### Listing mint or burn payments

Mint or burn payments are included in the regular payment history that is obtained when [Listing payments](./list_payments.md).

You can filter by token transaction type to only include mint, burn or transfer payments. Transfer payments are regular token payments that are not mint or burn payments.

```go
// Provide one or multiple of the following filters to
// the `PaymentDetailsFilter` field when listing payments
transferType := breez_sdk_spark.TokenTransactionTypeTransfer
mintType := breez_sdk_spark.TokenTransactionTypeMint
burnType := breez_sdk_spark.TokenTransactionTypeBurn
paymentDetailsTransferFilter := breez_sdk_spark.PaymentDetailsFilterToken{TxType: &transferType}
paymentDetailsMintFilter := breez_sdk_spark.PaymentDetailsFilterToken{TxType: &mintType}
paymentDetailsBurnFilter := breez_sdk_spark.PaymentDetailsFilterToken{TxType: &burnType}
```



## Query balance & metadata

Retrieve the current issued token balance and fetch the token metadata.

```go
tokenBalance, err := tokenIssuer.GetIssuerTokenBalance()
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}
log.Printf("Token balance: %v", tokenBalance.Balance)

tokenMetadata, err := tokenIssuer.GetIssuerTokenMetadata()
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}
log.Printf("Token ticker: %v", tokenMetadata.Ticker)
```



## Freeze and unfreeze tokens

Freeze and unfreeze tokens at a specific Spark address if the token metadata allows it.

```go
sparkAddress := "<spark address>"
// Freeze the tokens held at the specified Spark address
freezeRequest := breez_sdk_spark.FreezeIssuerTokenRequest{
	Address: sparkAddress,
}
freezeResponse, err := tokenIssuer.FreezeIssuerToken(freezeRequest)
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}

// Unfreeze the tokens held at the specified Spark address
unfreezeRequest := breez_sdk_spark.UnfreezeIssuerTokenRequest{
	Address: sparkAddress,
}
unfreezeResponse, err := tokenIssuer.UnfreezeIssuerToken(unfreezeRequest)
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}
```
