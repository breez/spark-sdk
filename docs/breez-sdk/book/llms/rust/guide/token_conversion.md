# Converting tokens

Token conversion enables payments to be made without holding the required asset by converting on-the-fly between Bitcoin and tokens using the Flashnet protocol.

## Fetching conversion limits

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.fetch_conversion_limits

Before performing a conversion, you can fetch the minimum amounts required for the conversion. The limits depend on the conversion direction:

- **Bitcoin to token**: Minimum Bitcoin amount (in satoshis) and minimum token amount to receive (in token base units)
- **Token to Bitcoin**: Minimum token amount (in token base units) and minimum Bitcoin amount to receive (in satoshis)

```rust
// Fetch limits for converting Bitcoin to a token
let response = sdk
    .fetch_conversion_limits(FetchConversionLimitsRequest {
        conversion_type: ConversionType::FromBitcoin,
        token_identifier: Some("<token identifier>".to_string()),
    })
    .await?;

if let Some(min_from) = response.min_from_amount {
    info!("Minimum BTC to convert: {} sats", min_from);
}
if let Some(min_to) = response.min_to_amount {
    info!("Minimum tokens to receive: {} base units", min_to);
}

// Fetch limits for converting a token to Bitcoin
let response = sdk
    .fetch_conversion_limits(FetchConversionLimitsRequest {
        conversion_type: ConversionType::ToBitcoin {
            from_token_identifier: "<token identifier>".to_string(),
        },
        token_identifier: None,
    })
    .await?;

if let Some(min_from) = response.min_from_amount {
    info!("Minimum tokens to convert: {} base units", min_from);
}
if let Some(min_to) = response.min_to_amount {
    info!("Minimum BTC to receive: {} sats", min_to);
}
```



**Developer note**

Amounts are denominated in satoshis for Bitcoin (1 BTC = 100,000,000 sats) and in token base units for tokens. Token base units depend on the token's decimal specification.

## Converting Bitcoin to tokens

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment

Token conversion enables payments of tokens like <a href="https://sparkscan.io/token/3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca?network=mainnet" target="_blank">USDB</a> to be made without holding the token, but instead using Bitcoin.

To do so, when preparing to send a payment, set the conversion options. The conversion will first calculate the Bitcoin amount needed to be converted into the token, convert Bitcoin into that token amount, and then finally complete the payment.

```rust
let payment_request = "<spark address or invoice>".to_string();
// Token identifier must match the invoice in case it specifies one.
let token_identifier = Some("<token identifier>".to_string());
// Set the amount of tokens you wish to send (in token base units).
let amount = Some(1_000);
// Set to use Bitcoin funds to pay via conversion
let optional_max_slippage_bps = Some(50);
let optional_completion_timeout_secs = Some(30);
let conversion_options = Some(ConversionOptions {
    conversion_type: ConversionType::FromBitcoin,
    max_slippage_bps: optional_max_slippage_bps,
    completion_timeout_secs: optional_completion_timeout_secs,
});

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: payment_request,
        },
        amount,
        token_identifier,
        conversion_options,
        fee_policy: None,
    })
    .await?;

// If the fees are acceptable, continue to send the token payment
if let Some(conversion_estimate) = &prepare_response.conversion_estimate {
    info!(
        "Estimated conversion: {} token units → {} sats",
        conversion_estimate.amount_in, conversion_estimate.amount_out
    );
    info!(
        "Estimated conversion fee: {} token units",
        conversion_estimate.fee
    );
}
```



**Developer note**

When a conversion fails due to exceeding the maximum slippage, the conversion will be refunded automatically.

**Developer note**

The conversion may result in some token balance remaining in the wallet after the payment is sent. This remaining balance is to account for slippage in the conversion.

## Converting tokens to Bitcoin

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment

Token conversion also enables Bitcoin payments to be made without holding the required Bitcoin, but instead using a supported token asset like <a href="https://sparkscan.io/token/3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca?network=mainnet" target="_blank">USDB</a>.

To do so, when preparing to send a payment, set the conversion options. The conversion will first calculate the amount needed to be converted into Bitcoin, convert the token into that Bitcoin amount, and then finally complete the payment.

```rust
let payment_request = "<payment request>".to_string();
// Set to use token funds to pay via conversion
let optional_max_slippage_bps = Some(50);
let optional_completion_timeout_secs = Some(30);
let conversion_options = Some(ConversionOptions {
    conversion_type: ConversionType::ToBitcoin {
        from_token_identifier: "<token identifier>".to_string(),
    },
    max_slippage_bps: optional_max_slippage_bps,
    completion_timeout_secs: optional_completion_timeout_secs,
});

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: payment_request,
        },
        amount: None,
        token_identifier: None,
        conversion_options,
        fee_policy: None,
    })
    .await?;

// If the fees are acceptable, continue to create the Send Payment
if let Some(conversion_estimate) = &prepare_response.conversion_estimate {
    info!(
        "Estimated conversion: {} token units → {} sats",
        conversion_estimate.amount_in, conversion_estimate.amount_out
    );
    info!(
        "Estimated conversion fee: {} token units",
        conversion_estimate.fee
    );
}
```



**Developer note**

When a conversion fails due to exceeding the maximum slippage, the conversion will be refunded automatically.

**Developer note**

The conversion may result in some Bitcoin remaining in the wallet after the payment is sent. This remaining Bitcoin is to account for slippage in the conversion.
