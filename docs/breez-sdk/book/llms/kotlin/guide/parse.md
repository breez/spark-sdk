# Parsing inputs

The SDK provides a versatile and extensible parsing module designed to process a wide range of input strings and return parsed data in various standardized formats.

Natively supported formats include: BOLT11 invoices, LNURLs of different types, Bitcoin addresses, Spark addresses, and others. For the complete list, consult the [API documentation](https://breez.github.io/spark-sdk/breez_sdk_spark/enum.InputType.html).

Cross-chain destinations on EVM, Solana, and Tron — bare addresses or chain-prefixed URIs — parse to `InputType.CrossChainAddress`, carrying the parsed address family along with any token contract address and amount embedded in the URI. Use the resulting `CrossChainAddressDetails` to discover available routes; see [Send USDC/USDT](./send_payment.md#usdc-usdt) for the send flow.

**Developer note**

The amounts returned from calling parse on Lightning based inputs (BOLT11, LNURL) are denominated in millisatoshi.

```kotlin
val input = "an input to be parsed..."

try {
    val inputType = sdk.parse(input)
    when (inputType) {
        is InputType.BitcoinAddress -> {
            println("Input is Bitcoin address ${inputType.v1.address}")
        }
        is InputType.Bolt11Invoice -> {
            val amountStr = inputType.v1.amountMsat?.toString() ?: "unknown"
            println("Input is BOLT11 invoice for $amountStr msats")
        }
        is InputType.LnurlPay -> {
            println(
                    "Input is LNURL-Pay/Lightning address accepting min/max " +
                            "${inputType.v1.minSendable}/${inputType.v1.maxSendable} msats}"
            )
        }
        is InputType.LnurlWithdraw -> {
            println(
                    "Input is LNURL-Withdraw for min/max " +
                            "${inputType.v1.minWithdrawable}/${inputType.v1.maxWithdrawable} msats"
            )
        }
        is InputType.SparkAddress -> {
            println("Input is Spark address ${inputType.v1.address}")
        }
        is InputType.SparkInvoice -> {
            val invoice = inputType.v1
            println("Input is Spark invoice:")
            if (invoice.tokenIdentifier != null) {
                println(
                        "  Amount: ${invoice.amount} base units of token " +
                                "with id ${invoice.tokenIdentifier}"
                )
            } else {
                println("  Amount: ${invoice.amount} sats")
            }

            if (invoice.description != null) {
                println("  Description: ${invoice.description}")
            }

            if (invoice.expiryTime != null) {
                println("  Expiry time: ${invoice.expiryTime}")
            }

            if (invoice.senderPublicKey != null) {
                println("  Sender public key: ${invoice.senderPublicKey}")
            }
        }
        is InputType.CrossChainAddress -> {
            val details = inputType.v1
            println(
                    "Input is cross-chain address ${details.address} " +
                            "(${details.addressFamily})"
            )
        }
        else -> {
            // Handle other input types
        }
    }
} catch (e: Exception) {
    // handle error
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

```kotlin
// Create the default config
val config = defaultConfig(Network.MAINNET)
config.apiKey = "<breez api key>"

// Configure external parsers
config.externalInputParsers = listOf(
    ExternalInputParser(
        providerId = "provider_a",
        inputRegex = "^provider_a",
        parserUrl = "https://parser-domain.com/parser?input=<input>"
    ),
    ExternalInputParser(
        providerId = "provider_b",
        inputRegex = "^provider_b",
        parserUrl = "https://parser-domain.com/parser?input=<input>"
    )
)
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
