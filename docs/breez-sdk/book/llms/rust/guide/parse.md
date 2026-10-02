# Parsing inputs

The SDK provides a versatile and extensible parsing module designed to process a wide range of input strings and return parsed data in various standardized formats.

Natively supported formats include: BOLT11 invoices, LNURLs of different types, Bitcoin addresses, Spark addresses, and others. For the complete list, consult the [API documentation](https://breez.github.io/spark-sdk/breez_sdk_spark/enum.InputType.html).

Cross-chain destinations on EVM, Solana, and Tron — bare addresses or chain-prefixed URIs — parse to `InputType::CrossChainAddress`, carrying the parsed address family along with any token contract address and amount embedded in the URI. Use the resulting `CrossChainAddressDetails` to discover available routes; see [Send USDC/USDT](./send_payment.md#usdc-usdt) for the send flow.

**Developer note**

The amounts returned from calling parse on Lightning based inputs (BOLT11, LNURL) are denominated in millisatoshi.

```rust
let input = "an input to be parsed...";

match sdk.parse(input).await? {
    InputType::BitcoinAddress(details) => {
        println!("Input is Bitcoin address {}", details.address);
    }
    InputType::Bolt11Invoice(details) => {
        println!(
            "Input is BOLT11 invoice for {} msats",
            details
                .amount_msat
                .map_or("unknown".to_string(), |a| a.to_string())
        );
    }
    InputType::LnurlPay(details) => {
        println!(
            "Input is LNURL-Pay/Lightning address accepting min/max {}/{} msats",
            details.min_sendable, details.max_sendable
        );
    }
    InputType::LnurlWithdraw(details) => {
        println!(
            "Input is LNURL-Withdraw for min/max {}/{} msats",
            details.min_withdrawable, details.max_withdrawable
        );
    }
    InputType::SparkAddress(details) => {
        println!("Input is Spark address {}", details.address);
    }
    InputType::SparkInvoice(invoice) => {
        println!("Input is Spark invoice:");
        if let Some(token_identifier) = &invoice.token_identifier {
            println!(
                "  Amount: {:?} base units of token with id {}",
                invoice.amount, token_identifier
            );
        } else {
            println!("  Amount: {:?} sats", invoice.amount);
        }

        if let Some(description) = &invoice.description {
            println!("  Description: {}", description);
        }

        if let Some(expiry_time) = invoice.expiry_time {
            println!("  Expiry time: {}", expiry_time);
        }

        if let Some(sender_public_key) = &invoice.sender_public_key {
            println!("  Sender public key: {}", sender_public_key);
        }
    }
    InputType::CrossChainAddress(details) => {
        println!(
            "Input is cross-chain address {} ({:?})",
            details.address, details.address_family
        );
    }
    // Other input types are available
    _ => {}
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

```rust
// Create the default config
let mut config = default_config(Network::Mainnet);
config.api_key = Some("<breez api key>".to_string());

// Configure external parsers
config.external_input_parsers = Some(vec![
    ExternalInputParser {
        provider_id: "provider_a".to_string(),
        input_regex: "^provider_a".to_string(),
        parser_url: "https://parser-domain.com/parser?input=<input>".to_string(),
    },
    ExternalInputParser {
        provider_id: "provider_b".to_string(),
        input_regex: "^provider_b".to_string(),
        parser_url: "https://parser-domain.com/parser?input=<input>".to_string(),
    },
]);
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
