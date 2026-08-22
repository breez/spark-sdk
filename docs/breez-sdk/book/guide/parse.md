# Parsing inputs

The SDK provides a versatile and extensible parsing module designed to process a wide range of input strings and return parsed data in various standardized formats.

Natively supported formats include: BOLT11 invoices, LNURLs of different types, Bitcoin addresses, Spark addresses, and others. For the complete list, consult the [API documentation](https://breez.github.io/spark-sdk/breez_sdk_spark/enum.InputType.html).

Cross-chain destinations on EVM, Solana, and Tron — bare addresses or chain-prefixed URIs — parse to `InputType::CrossChainAddress`, carrying the parsed address family along with any token contract address and amount embedded in the URI. Use the resulting `CrossChainAddressDetails` to discover available routes; see [Send USDC/USDT](./send_payment.md#usdc-usdt) for the send flow.

**Developer note**

The amounts returned from calling parse on Lightning based inputs (BOLT11, LNURL) are denominated in millisatoshi.

## Rust

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

## Swift

```swift
let input = "an input to be parsed..."

do {
    let inputType = try await sdk.parse(input: input)
    switch inputType {
    case .bitcoinAddress(v1: let details):
        print("Input is Bitcoin address \(details.address)")

    case .bolt11Invoice(v1: let details):
        let amount = details.amountMsat.map { String($0) } ?? "unknown"
        print("Input is BOLT11 invoice for \(amount) msats")

    case .lnurlPay(v1: let details):
        print(
            "Input is LNURL-Pay/Lightning address accepting min/max "
                + "\(details.minSendable)/\(details.maxSendable) msats)"
        )
    case .lnurlWithdraw(v1: let details):
        print(
            "Input is LNURL-Withdraw for min/max "
                + "\(details.minWithdrawable)/\(details.maxWithdrawable) msats"
        )

    case .sparkAddress(v1: let details):
        print("Input is Spark address \(details.address)")

    case .sparkInvoice(v1: let invoice):
        print("Input is Spark invoice:")
        if let tokenIdentifier = invoice.tokenIdentifier {
            print("  Amount: \(invoice.amount) base units of token with id \(tokenIdentifier)")
        } else {
            print("  Amount: \(invoice.amount) sats")
        }

        if let description = invoice.description {
            print("  Description: \(description)")
        }

        if let expiryTime = invoice.expiryTime {
            print("  Expiry time: \(Date(timeIntervalSince1970: TimeInterval(expiryTime)))")
        }

        if let senderPublicKey = invoice.senderPublicKey {
            print("  Sender public key: \(senderPublicKey)")
        }

    case .crossChainAddress(v1: let details):
        print("Input is cross-chain address \(details.address) (\(details.addressFamily))")

    default:
        break  // Other input types are available
    }
} catch {
    print("Failed to parse input: \(error)")
}
```

## Kotlin

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

## C#

```csharp
var inputStr = "an input to be parsed...";

var parsedInput = await sdk.Parse(input: inputStr);
switch (parsedInput)
{
    case InputType.BitcoinAddress bitcoinAddress:
        var details = bitcoinAddress.v1;
        Console.WriteLine($"Input is Bitcoin address {details.address}");
        break;

    case InputType.Bolt11Invoice bolt11:
        var bolt11Details = bolt11.v1;
        var amount = bolt11Details.amountMsat.HasValue
            ? bolt11Details.amountMsat.Value.ToString()
            : "unknown";
        Console.WriteLine($"Input is BOLT11 invoice for {amount} msats");
        break;

    case InputType.LnurlPay lnurlPay:
        var lnurlPayDetails = lnurlPay.v1;
        Console.WriteLine($"Input is LNURL-Pay/Lightning address accepting " +
                        $"min/max {lnurlPayDetails.minSendable}/" +
                        $"{lnurlPayDetails.maxSendable} msats");
        break;

    case InputType.LnurlWithdraw lnurlWithdraw:
        var lnurlWithdrawDetails = lnurlWithdraw.v1;
        Console.WriteLine($"Input is LNURL-Withdraw for min/max " +
                        $"{lnurlWithdrawDetails.minWithdrawable}/" +
                        $"{lnurlWithdrawDetails.maxWithdrawable} msats");
        break;

    case InputType.SparkAddress sparkAddress:
        var sparkAddressDetails = sparkAddress.v1;
        Console.WriteLine($"Input is Spark address {sparkAddressDetails.address}");
        break;

    case InputType.SparkInvoice sparkInvoice:
        var invoice = sparkInvoice.v1;
        Console.WriteLine("Input is Spark invoice:");
        if (invoice.tokenIdentifier != null)
        {
            Console.WriteLine($"  Amount: {invoice.amount} base units of " +
                            $"token with id {invoice.tokenIdentifier}");
        }
        else
        {
            Console.WriteLine($"  Amount: {invoice.amount} sats");
        }

        if (invoice.description != null)
        {
            Console.WriteLine($"  Description: {invoice.description}");
        }

        if (invoice.expiryTime.HasValue)
        {
            Console.WriteLine($"  Expiry time: {invoice.expiryTime}");
        }

        if (invoice.senderPublicKey != null)
        {
            Console.WriteLine($"  Sender public key: {invoice.senderPublicKey}");
        }
        break;

    case InputType.CrossChainAddress crossChainAddress:
        var crossChainDetails = crossChainAddress.v1;
        Console.WriteLine($"Input is cross-chain address {crossChainDetails.address} " +
                        $"({crossChainDetails.addressFamily})");
        break;

        // Other input types are available
}
```

## Javascript (Wasm)

```typescript
const input = 'an input to be parsed...'

const parsed = await sdk.parse(input)

switch (parsed.type) {
  case 'bitcoinAddress':
    console.log(`Input is Bitcoin address ${parsed.address}`)
    break

  case 'bolt11Invoice':
    console.log(
      `Input is BOLT11 invoice for ${
        parsed.amountMsat != null ? parsed.amountMsat.toString() : 'unknown'
      } msats`
    )
    break

  case 'lnurlPay':
    console.log(
      'Input is LNURL-Pay/Lightning address accepting min/max ' +
      `${parsed.minSendable}/${parsed.maxSendable} msats`
    )
    break

  case 'lnurlWithdraw':
    console.log(
      'Input is LNURL-Withdraw for min/max ' +
      `${parsed.minWithdrawable}/${parsed.maxWithdrawable} msats`
    )
    break

  case 'sparkAddress':
    console.log(`Input is Spark address ${parsed.address}`)
    break

  case 'sparkInvoice':
    console.log('Input is Spark invoice:')
    if (parsed.tokenIdentifier != null) {
      console.log(
        `  Amount: ${parsed.amount} base units of token with id ${parsed.tokenIdentifier}`
      )
    } else {
      console.log(`  Amount: ${parsed.amount} sats`)
    }

    if (parsed.description != null) {
      console.log(`  Description: ${parsed.description}`)
    }

    if (parsed.expiryTime != null) {
      console.log(`  Expiry time: ${new Date(Number(parsed.expiryTime) * 1000).toISOString()}`)
    }

    if (parsed.senderPublicKey != null) {
      console.log(`  Sender public key: ${parsed.senderPublicKey}`)
    }
    break

  case 'crossChainAddress':
    console.log(`Input is cross-chain address ${parsed.address} (${parsed.addressFamily})`)
    break

  default:
    // Other input types are available
    break
}
```

## React Native

```typescript
const inputStr = 'an input to be parsed...'

const input = await sdk.parse(inputStr)

if (input.tag === InputType_Tags.BitcoinAddress) {
  console.log(`Input is Bitcoin address ${input.inner[0].address}`)
} else if (input.tag === InputType_Tags.Bolt11Invoice) {
  console.log(
    `Input is BOLT11 invoice for ${
      input.inner[0].amountMsat != null ? input.inner[0].amountMsat.toString() : 'unknown'
    } msats`
  )
} else if (input.tag === InputType_Tags.LnurlPay) {
  console.log(
    'Input is LNURL-Pay/Lightning address accepting min/max ' +
      `${input.inner[0].minSendable}/${input.inner[0].maxSendable} msats`
  )
} else if (input.tag === InputType_Tags.LnurlWithdraw) {
  console.log(
    'Input is LNURL-Withdraw for min/max ' +
      `${input.inner[0].minWithdrawable}/${input.inner[0].maxWithdrawable} msats`
  )
} else if (input.tag === InputType_Tags.SparkAddress) {
  console.log(`Input is Spark address ${input.inner[0].address}`)
} else if (input.tag === InputType_Tags.SparkInvoice) {
  const invoice = input.inner[0]
  console.log('Input is Spark invoice:')
  if (invoice.tokenIdentifier != null) {
    console.log(
      `  Amount: ${invoice.amount} base units of token with id ${invoice.tokenIdentifier}`
    )
  } else {
    console.log(`  Amount: ${invoice.amount} sats`)
  }

  if (invoice.description != null) {
    console.log(`  Description: ${invoice.description}`)
  }

  if (invoice.expiryTime != null) {
    console.log(`  Expiry time: ${new Date(Number(invoice.expiryTime) * 1000).toISOString()}`)
  }

  if (invoice.senderPublicKey != null) {
    console.log(`  Sender public key: ${invoice.senderPublicKey}`)
  }
} else if (input.tag === InputType_Tags.CrossChainAddress) {
  const details = input.inner[0]
  console.log(`Input is cross-chain address ${details.address} (${details.addressFamily})`)
} else {
  // Other input types are available
}
```

## Flutter

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

## Python

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

## Go

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

#### Rust

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

#### Swift

```swift
// Create the default config
var config = defaultConfig(network: Network.mainnet)
config.apiKey = "<breez api key>"

// Configure external parsers
config.externalInputParsers = [
    ExternalInputParser(
        providerId: "provider_a",
        inputRegex: "^provider_a",
        parserUrl: "https://parser-domain.com/parser?input=<input>"
    ),
    ExternalInputParser(
        providerId: "provider_b",
        inputRegex: "^provider_b",
        parserUrl: "https://parser-domain.com/parser?input=<input>"
    ),
]
```

#### Kotlin

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

#### C#

```csharp
// Create the default config
var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
{
    apiKey = "<breez api key>",
    externalInputParsers = new ExternalInputParser[]
    {
    new ExternalInputParser(
        providerId: "provider_a",
        inputRegex: "^provider_a",
        parserUrl: "https://parser-domain.com/parser?input=<input>"
    ),
    new ExternalInputParser(
        providerId: "provider_b",
        inputRegex: "^provider_b",
        parserUrl: "https://parser-domain.com/parser?input=<input>"
    )
    }
};
```

#### Javascript (Wasm)

```typescript
// Create the default config
const config = defaultConfig('mainnet')
config.apiKey = '<breez api key>'

// Configure external parsers
config.externalInputParsers = [
  {
    providerId: 'provider_a',
    inputRegex: '^provider_a',
    parserUrl: 'https://parser-domain.com/parser?input=<input>'
  },
  {
    providerId: 'provider_b',
    inputRegex: '^provider_b',
    parserUrl: 'https://parser-domain.com/parser?input=<input>'
  }
]
```

#### React Native

```typescript
// Create the default config
const config = defaultConfig(Network.Mainnet)
config.apiKey = '<breez api key>'

// Configure external parsers
config.externalInputParsers = [
  {
    providerId: 'provider_a',
    inputRegex: '^provider_a',
    parserUrl: 'https://parser-domain.com/parser?input=<input>'
  },
  {
    providerId: 'provider_b',
    inputRegex: '^provider_b',
    parserUrl: 'https://parser-domain.com/parser?input=<input>'
  }
]
```

#### Flutter

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

#### Python

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

#### Go

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

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
