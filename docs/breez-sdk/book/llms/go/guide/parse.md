# Parsing inputs

The SDK provides a versatile and extensible parsing module designed to process a wide range of input strings and return parsed data in various standardized formats.

Natively supported formats include: BOLT11 invoices, LNURLs of different types, Bitcoin addresses, Spark addresses, and others. For the complete list, consult the [API documentation](https://breez.github.io/spark-sdk/breez_sdk_spark/enum.InputType.html).

Cross-chain destinations on EVM, Solana, and Tron — bare addresses or chain-prefixed URIs — parse to `InputTypeCrossChainAddress`, carrying the parsed address family along with any token contract address and amount embedded in the URI. Use the resulting `CrossChainAddressDetails` to discover available routes; see [Send USDC/USDT](./send_payment.md#usdc-usdt) for the send flow.

**Developer note**

The amounts returned from calling parse on Lightning based inputs (BOLT11, LNURL) are denominated in millisatoshi.

```go
inputStr := "an input to be parsed..."

input, err := sdk.Parse(inputStr)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

switch inputType := input.(type) {
case breez_sdk_spark.InputTypeBitcoinAddress:
	log.Printf("Input is Bitcoin address %s", inputType.Field0.Address)

case breez_sdk_spark.InputTypeBolt11Invoice:
	amount := "unknown"
	if inputType.Field0.AmountMsat != nil {
		amount = strconv.FormatUint(*inputType.Field0.AmountMsat, 10)
	}
	log.Printf("Input is BOLT11 invoice for %s msats", amount)

case breez_sdk_spark.InputTypeLnurlPay:
	log.Printf("Input is LNURL-Pay/Lightning address accepting min/max %d/%d msats",
		inputType.Field0.MinSendable, inputType.Field0.MaxSendable)

case breez_sdk_spark.InputTypeLnurlWithdraw:
	log.Printf("Input is LNURL-Withdraw for min/max %d/%d msats",
		inputType.Field0.MinWithdrawable, inputType.Field0.MaxWithdrawable)

case breez_sdk_spark.InputTypeSparkAddress:
	log.Printf("Input is Spark address %s", inputType.Field0.Address)

case breez_sdk_spark.InputTypeSparkInvoice:
	invoice := inputType.Field0
	log.Println("Input is Spark invoice:")
	if invoice.TokenIdentifier != nil {
		log.Printf(
			"  Amount: %d base units of token with id %s",
			invoice.Amount,
			*invoice.TokenIdentifier,
		)
	} else {
		log.Printf("  Amount: %d sats", invoice.Amount)
	}

	if invoice.Description != nil {
		log.Printf("  Description: %s", *invoice.Description)
	}

	if invoice.ExpiryTime != nil {
		log.Printf("  Expiry time: %d", *invoice.ExpiryTime)
	}

	if invoice.SenderPublicKey != nil {
		log.Printf("  Sender public key: %s", *invoice.SenderPublicKey)
	}

case breez_sdk_spark.InputTypeCrossChainAddress:
	details := inputType.Field0
	log.Printf(
		"Input is cross-chain address %s (%v)",
		details.Address,
		details.AddressFamily,
	)

default:
	// Other input types are available
}
```



## Supporting other input formats

The parsing module can be extended using external input parsers provided in the SDK configuration. These will be used when the input is not recognized.

You can implement and provide your own parsers, or use existing public ones.

### Configuring external parsers

Configuring external parsers can only be done before [initializing](initializing.md#basic-initialization) and the config cannot be changed through the lifetime of the connection.

Multiple parsers can be configured, and each one is defined by:

- **Provider ID**: an arbitrary id to identify the provider input type
- **Input regex**: a regex pattern that should reliably match all inputs that this parser can process, even if it may also match some invalid inputs
- **Parser URL**: an URL containing the placeholder `<input>`

When parsing an input that isn't recognized as one of the native input types, the SDK will check if the input conforms to any of the external parsers regex expressions. If so, it will make an HTTP `GET` request to the provided URL, replacing the placeholder with the input. If the input is recognized, the response should include in its body a string that can be parsed into one of the natively supported types.

```go
// Create the default config
apiKey := "<breez api key>"
config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
config.ApiKey = &apiKey

// Configure external parsers
parsers := []breez_sdk_spark.ExternalInputParser{
	{
		ProviderId: "provider_a",
		InputRegex: "^provider_a",
		ParserUrl:  "https://parser-domain.com/parser?input=<input>",
	},
	{
		ProviderId: "provider_b",
		InputRegex: "^provider_b",
		ParserUrl:  "https://parser-domain.com/parser?input=<input>",
	},
}
config.ExternalInputParsers = &parsers
```



### Public external parsers

- [**PicknPay QRs**](https://www.pnp.co.za/)
  - Maintainer: [MoneyBadger](https://www.moneybadger.co.za/)
  - Regex: `(.*)(za.co.electrum.picknpay)(.*)`
  - URL: `https://cryptoqr.net/.well-known/lnurlp/<input>`
  - More info: [support+breezsdk@moneybadger.co.za](mailto:support+breezsdk@moneybadger.co.za)
- [**Bootlegger QRs**](https://www.bootlegger.coffee/)
  - Maintainer: [MoneyBadger](https://www.moneybadger.co.za/)
  - Regex: `(.*)(wigroup\.co|yoyogroup\.co)(.*)`
  - URL: `https://cryptoqr.net/.well-known/lnurlw/<input>`
  - More info: [support+breezsdk@moneybadger.co.za](mailto:support+breezsdk@moneybadger.co.za)

### Default external parsers

The SDK ships with some embedded default external parsers. If you prefer not to use them, you can disable them in the SDK's configuration. See the available default parsers in the [API Documentation](https://breez.github.io/spark-sdk/breez_sdk_spark/constant.DEFAULT_EXTERNAL_INPUT_PARSERS.html) by checking the source of the constant.
