# Parsing inputs

The SDK provides a versatile and extensible parsing module designed to process a wide range of input strings and return parsed data in various standardized formats.

Natively supported formats include: BOLT11 invoices, LNURLs of different types, Bitcoin addresses, Spark addresses, and others. For the complete list, consult the [API documentation](https://breez.github.io/spark-sdk/breez_sdk_spark/enum.InputType.html).

Cross-chain destinations on EVM, Solana, and Tron — bare addresses or chain-prefixed URIs — parse to `InputType.CROSS_CHAIN_ADDRESS`, carrying the parsed address family along with any token contract address and amount embedded in the URI. Use the resulting `CrossChainAddressDetails` to discover available routes; see [Send USDC/USDT](./send_payment.md#usdc-usdt) for the send flow.

**Developer note**

The amounts returned from calling parse on Lightning based inputs (BOLT11, LNURL) are denominated in millisatoshi.

```python
input_str = "an input to be parsed..."

try:
    parsed_input = await sdk.parse(input=input_str)
    if isinstance(parsed_input, InputType.BITCOIN_ADDRESS):
        details = parsed_input[0]
        logging.debug(f"Input is Bitcoin address {details.address}")
    elif isinstance(parsed_input, InputType.BOLT11_INVOICE):
        details = parsed_input[0]
        amount = "unknown"
        if details.amount_msat:
            amount = str(details.amount_msat)
        logging.debug(f"Input is BOLT11 invoice for {amount} msats")
    elif isinstance(parsed_input, InputType.LNURL_PAY):
        details = parsed_input[0]
        logging.debug(
            f"Input is LNURL-Pay/Lightning address accepting "
            f"min/max {details.min_sendable}/{details.max_sendable} msats"
        )
    elif isinstance(parsed_input, InputType.LNURL_WITHDRAW):
        details = parsed_input[0]
        logging.debug(
            f"Input is LNURL-Withdraw for min/max "
            f"{details.min_withdrawable}/{details.max_withdrawable} msats"
        )
    elif isinstance(parsed_input, InputType.SPARK_ADDRESS):
        details = parsed_input[0]
        logging.debug(f"Input is Spark address {details.address}")
    elif isinstance(parsed_input, InputType.SPARK_INVOICE):
        invoice = parsed_input[0]
        logging.debug("Input is Spark invoice:")
        if invoice.token_identifier:
            logging.debug(f"  Amount: {invoice.amount} base units of "
            f"token with id {invoice.token_identifier}")
        else:
            logging.debug(f"  Amount: {invoice.amount} sats")

        if invoice.description:
            logging.debug(f"  Description: {invoice.description}")

        if invoice.expiry_time:
            logging.debug(f"  Expiry time: {invoice.expiry_time}")

        if invoice.sender_public_key:
            logging.debug(f"  Sender public key: {invoice.sender_public_key}")
    elif isinstance(parsed_input, InputType.CROSS_CHAIN_ADDRESS):
        details = parsed_input[0]
        logging.debug(
            f"Input is cross-chain address {details.address} ({details.address_family})"
        )
    # Other input types are available
except Exception as error:
    logging.error(error)
    raise
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

```python
# Create the default config
config = default_config(network=Network.MAINNET)
config.api_key = "<breez api key>"

# Configure external parsers
config.external_input_parsers = [
    ExternalInputParser(
        provider_id="provider_a",
        input_regex="^provider_a",
        parser_url="https://parser-domain.com/parser?input=<input>"
    ),
    ExternalInputParser(
        provider_id="provider_b",
        input_regex="^provider_b",
        parser_url="https://parser-domain.com/parser?input=<input>"
    )
]
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
