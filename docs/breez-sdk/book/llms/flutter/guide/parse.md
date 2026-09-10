# Parsing inputs

The SDK provides a versatile and extensible parsing module designed to process a wide range of input strings and return parsed data in various standardized formats.

Natively supported formats include: BOLT11 invoices, LNURLs of different types, Bitcoin addresses, Spark addresses, and others. For the complete list, consult the [API documentation](https://breez.github.io/spark-sdk/breez_sdk_spark/enum.InputType.html).

Cross-chain destinations on EVM, Solana, and Tron — bare addresses or chain-prefixed URIs — parse to `InputType.CrossChainAddress`, carrying the parsed address family along with any token contract address and amount embedded in the URI. Use the resulting `CrossChainAddressDetails` to discover available routes; see [Send USDC/USDT](./send_payment.md#usdc-usdt) for the send flow.

**Developer note**

The amounts returned from calling parse on Lightning based inputs (BOLT11, LNURL) are denominated in millisatoshi.

```dart
String input = "an input to be parsed...";

InputType inputType = await sdk.parse(input: input);
if (inputType is InputType_BitcoinAddress) {
  print("Input is Bitcoin address ${inputType.field0.address}");
} else if (inputType is InputType_Bolt11Invoice) {
  String amountStr = inputType.field0.amountMsat != null
      ? inputType.field0.amountMsat.toString()
      : "unknown";
  print("Input is BOLT11 invoice for $amountStr msats");
} else if (inputType is InputType_LnurlPay) {
  print("Input is LNURL-Pay/Lightning address accepting min/max "
      "${inputType.field0.minSendable}/${inputType.field0.maxSendable} msats");
} else if (inputType is InputType_LnurlWithdraw) {
  print("Input is LNURL-Withdraw for min/max "
      "${inputType.field0.minWithdrawable}/${inputType.field0.maxWithdrawable} msats");
} else if (inputType is InputType_SparkAddress) {
  print("Input is Spark address ${inputType.field0.address}");
} else if (inputType is InputType_SparkInvoice) {
  var invoice = inputType.field0;
  print("Input is Spark invoice:");
  if (invoice.tokenIdentifier != null) {
    print("  Amount: ${invoice.amount} base units of token with id ${invoice.tokenIdentifier}");
  } else {
    print("  Amount: ${invoice.amount} sats");
  }

  if (invoice.description != null) {
    print("  Description: ${invoice.description}");
  }

  if (invoice.expiryTime != null) {
    print("  Expiry time: "
        "${DateTime.fromMillisecondsSinceEpoch(invoice.expiryTime!.toInt() * 1000)}");
  }

  if (invoice.senderPublicKey != null) {
    print("  Sender public key: ${invoice.senderPublicKey}");
  }
} else if (inputType is InputType_CrossChainAddress) {
  var details = inputType.field0;
  print("Input is cross-chain address ${details.address} (${details.addressFamily})");
} else {
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

```dart
// Create the default config
Config config = defaultConfig(network: Network.mainnet)
    .copyWith(apiKey: "<breez api key>");

config = config.copyWith(
  externalInputParsers: [
    ExternalInputParser(
      providerId: "provider_a",
      inputRegex: "^provider_a",
      parserUrl: "https://parser-domain.com/parser?input=<input>",
    ),
    ExternalInputParser(
      providerId: "provider_b",
      inputRegex: "^provider_b",
      parserUrl: "https://parser-domain.com/parser?input=<input>",
    ),
  ],
);
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
