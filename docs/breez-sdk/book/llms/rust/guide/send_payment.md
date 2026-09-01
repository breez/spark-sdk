# Sending payments

Once the SDK is initialized, you can directly begin sending payments. The send process takes two steps:

1. [Preparing the Payment](send_payment.md#preparing-payments)
2. [Sending the Payment](send_payment.md#sending-payments)

For sending payments via LNURL, see [LNURL-Pay](lnurl_pay.md).

## Preparing Payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment

During the prepare step, the SDK ensures that the inputs are valid with respect to the payment request type,
and also returns the fees related to the payment so they can be confirmed.

The payment request field supports Lightning invoices, Bitcoin addresses, Spark addresses and Spark invoices.

**Developer note**

Payments can be sent without holding Bitcoin by converting on-the-fly as a step before sending a payment. See <a href="./token_conversion.md">Converting tokens</a> for more information.

### Lightning

#### BOLT11 invoice

For BOLT11 invoices the amount can be optionally set. It is only required if the invoice doesn't specify an amount. If the invoice specifies an amount, providing a different amount is not supported.

If the invoice also contains a Spark address, the payment can be sent directly via a Spark transfer instead. When this is the case, the prepare response includes the Spark transfer fee. Note that only one fee is paid: either the Lightning fee or the Spark transfer fee, depending on which payment method is ultimately used. See [Lightning](send_payment.md#lightning-1) for how to select the payment method.

```rust
let payment_request = "<bolt11 invoice>".to_string();
// Optionally set the amount you wish to pay the receiver
let optional_amount_sats = Some(5_000);

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: payment_request,
        },
        amount: optional_amount_sats,
        token_identifier: None,
        conversion_options: None,
        fee_policy: None,
    })
    .await?;

// If the fees are acceptable, continue to create the Send Payment
if let SendPaymentMethod::Bolt11Invoice {
    spark_transfer_fee_sats,
    lightning_fee_sats,
    ..
} = prepare_response.payment_method
{
    // Fees to pay via Lightning
    info!("Lightning Fees: {lightning_fee_sats} sats");
    // Or fees to pay (if available) via a Spark transfer
    info!("Spark Transfer Fees: {spark_transfer_fee_sats:?} sats");
}
```



### Bitcoin

For Bitcoin addresses, the amount must be set in the request. The prepare response includes fee quotes for three payment speeds: Slow, Medium, and Fast.

```rust
let payment_request = "<bitcoin address>".to_string();
// Set the amount you wish to pay the receiver
let amount_sats = Some(50_000);

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: payment_request,
        },
        amount: amount_sats,
        token_identifier: None,
        conversion_options: None,
        fee_policy: None,
    })
    .await?;

// Review the fee quote for each confirmation speed
if let SendPaymentMethod::BitcoinAddress { fee_quote, .. } = &prepare_response.payment_method {
    info!("Slow fee: {} sats", fee_quote.speed_slow.total_fee_sat());
    info!(
        "Medium fee: {} sats",
        fee_quote.speed_medium.total_fee_sat()
    );
    info!("Fast fee: {} sats", fee_quote.speed_fast.total_fee_sat());
}
```



### Spark

#### Spark address

For Spark addresses, the amount must be set in the request. Sending to a Spark address uses a direct Spark transfer.

```rust
let payment_request = "<spark address>".to_string();
// Set the amount you wish to pay the receiver
let amount_sats = Some(50_000);

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: payment_request,
        },
        amount: amount_sats,
        token_identifier: None,
        conversion_options: None,
        fee_policy: None,
    })
    .await?;

// If the fees are acceptable, continue to create the Send Payment
if let SendPaymentMethod::SparkAddress { fee, .. } = prepare_response.payment_method {
    info!("Fees: {} sats", fee);
}
```



#### Spark invoice

For Spark invoices, the amount can be optionally set. It is only required if the invoice doesn't specify an amount. If the invoice specifies an amount, providing a different amount is not supported.

**Developer note**

Spark invoices may require a token (non-Bitcoin) as the payment asset. To determine the requirements of a Spark invoice and any restrictions it may impose, see the <a href="./parse.md">Parsing inputs</a> page. To learn more about tokens, see the <a href="./tokens.md">Handling tokens</a> page.

```rust
let payment_request = "<spark invoice>".to_string();
// Optionally set the amount you wish to pay the receiver
let optional_amount_sats = Some(50_000);

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: payment_request,
        },
        amount: optional_amount_sats,
        token_identifier: None,
        conversion_options: None,
        fee_policy: None,
    })
    .await?;

// If the fees are acceptable, continue to create the Send Payment
if let SendPaymentMethod::SparkInvoice { fee, .. } = prepare_response.payment_method {
    info!("Fees: {} sats", fee);
}
```



### USDC/USDT

Send USDC or USDT from a Spark wallet to a recipient on one of several supported chains: Ethereum-family chains (Arbitrum, Base, and similar EVM networks), Solana, and Tron. The source on the Spark side is BTC sats or USDB. This feature must be enabled in [the SDK configuration](./config.md#send-usdc-usdt) before using. See [Send USDC/USDT](./cross_chain.md) for provider details and the status lifecycle.

After [parsing](./parse.md) the recipient address into `InputType::CrossChainAddress`, call `get_cross_chain_routes` with `CrossChainRouteFilter::Send` carrying the parsed `CrossChainAddressDetails`. The returned `CrossChainRoutePair`s name the provider, destination chain and asset, decimals, optional token contract address, and which source assets (BTC sats or USDB) each route accepts.

```rust
let input = "<recipient address>";
let InputType::CrossChainAddress(address_details) = sdk.parse(input).await? else {
    anyhow::bail!("Not a cross-chain address");
};

let routes = sdk
    .get_cross_chain_routes(&CrossChainRouteFilter::Send {
        address_details: address_details.clone(),
    })
    .await?;

for route in &routes {
    info!(
        "Route via {:?}: {}/{}",
        route.provider, route.chain, route.asset
    );
}
```



Build `PaymentRequest::CrossChain` with the recipient address, the chosen route, and an optional `max_slippage_bps` (10 to 500 basis points). The amount on the prepare request is denominated in the source asset's base units: sats for a BTC source, USDB base units for a USDB source.

The prepare response carries a quote `expires_at` timestamp. Re-prepare and pick a fresh route if it lapses before send.

```rust
// Optionally set the maximum slippage in basis points (10 to 500)
let optional_max_slippage_bps = Some(100);

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::CrossChain {
            address: address_details.address.clone(),
            route,
            max_slippage_bps: optional_max_slippage_bps,
            target_overpay_bps: None,
        },
        amount: Some(50_000),
        token_identifier: None,
        conversion_options: None,
        fee_policy: None,
    })
    .await?;

if let SendPaymentMethod::CrossChainAddress {
    amount_in,
    estimated_out,
    fee_amount,
    expires_at,
    ..
} = &prepare_response.payment_method
{
    info!("Amount in: {amount_in}");
    info!("Estimated out: {estimated_out}");
    info!("Provider fee: {fee_amount}");
    info!("Quote expires at: {expires_at}");
}
```



## Fee Policy

By default, fees are added on top of the amount (`FeePolicy::FeesExcluded`). Use `FeePolicy::FeesIncluded` to deduct fees from the amount instead—the receiver gets the amount minus fees.

This is particularly useful when you want to spend your entire balance in a single payment—simply provide your full balance as the amount. Note: `FeePolicy::FeesIncluded` is not compatible with payment requests that specify an amount (e.g., BOLT11 invoices and Spark invoices with amount).

```rust
// By default (FeePolicy::FeesExcluded), fees are added on top of the amount.
// Use FeePolicy::FeesIncluded to deduct fees from the amount instead.
// The receiver gets amount minus fees.
let payment_request = "<payment request>".to_string();
let amount_sats = Some(50_000);

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: payment_request,
        },
        amount: amount_sats,
        token_identifier: None,
        conversion_options: None,
        fee_policy: Some(FeePolicy::FeesIncluded),
    })
    .await?;

// The response shows the fee policy used
info!("Fee policy: {:?}", prepare_response.fee_policy);
info!("Amount: {}", prepare_response.amount);
// The receiver gets amount - fees (fees are available in prepare_response.payment_method)
```



When [stable balance](./stable_balance.md) is active, you can send your entire wallet balance — both the token balance and any remaining sats — by combining `FeePolicy::FeesIncluded` with `ConversionType::ToBitcoin` conversion options. See [Sending entire balance](./stable_balance.md#sending-entire-balance) for details.

```rust
let payment_request = "<payment request>".to_string();
let token_identifier = "<token identifier>".to_string();

let info = sdk
    .get_info(GetInfoRequest {
        ensure_synced: Some(false),
    })
    .await?;

let token_balance = info
    .token_balances
    .get(&token_identifier)
    .ok_or_else(|| anyhow::anyhow!("Token balance not found"))?;

let conversion_options = Some(ConversionOptions {
    conversion_type: ConversionType::ToBitcoin {
        from_token_identifier: token_identifier.clone(),
    },
    max_slippage_bps: None,
    completion_timeout_secs: None,
});

let prepare_response = sdk
    .prepare_send_payment(PrepareSendPaymentRequest {
        payment_request: PaymentRequest::Input {
            input: payment_request,
        },
        amount: Some(token_balance.balance),
        token_identifier: Some(token_identifier),
        conversion_options,
        fee_policy: Some(FeePolicy::FeesIncluded),
    })
    .await?;

// The response amount is the estimated total sats available
// (converted sats + existing sat balance)
info!("Total sats available: {}", prepare_response.amount);

if let Some(conversion_estimate) = &prepare_response.conversion_estimate {
    info!(
        "Converting {} token units → ~{} sats",
        conversion_estimate.amount_in, conversion_estimate.amount_out
    );
    info!("Conversion fee: {} token units", conversion_estimate.fee);
}
```



## Sending Payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.send_payment

Once the payment has been prepared and the fees are accepted, the payment can be sent by passing:

- **Prepare Response** - The response from the [Preparing the Payment](send_payment.md#preparing-payments) step.
- **Options** - Any payment method specific options for the payment (see below).
- **Idempotency Key** - An optional UUID that identifies the payment. If set, providing the same idempotency key for multiple requests will ensure that only one payment is made.

### Lightning

In the optional send payment options for BOLT11 invoices, you can set:

- **Prefer Spark** - Set the preference to use Spark to transfer the payment if the invoice contains a Spark address. By default, using Spark transfers are disabled.
- **Completion Timeout** - By default, this function returns immediately. You can override this behavior by specifying a completion timeout in seconds. If the timeout is reached, a pending payment object is returned. If the payment completes within the timeout, the completed payment object is returned.

```rust
let options = Some(SendPaymentOptions::Bolt11Invoice {
    prefer_spark: false,
    completion_timeout_secs: Some(10),
});
let optional_idempotency_key = Some("<idempotency key uuid>".to_string());
let send_response = sdk
    .send_payment(SendPaymentRequest {
        prepare_response,
        options,
        idempotency_key: optional_idempotency_key,
    })
    .await?;
let payment = send_response.payment;
info!("Payment: {payment:?}");
```



### Bitcoin

In the optional send payment options for Bitcoin addresses, you can set:

- **Confirmation Speed** - The priority that the Bitcoin transaction confirms, that also effects the fee paid. By default, it is set to Fast.

```rust
// Select the confirmation speed for the on-chain transaction
let options = Some(SendPaymentOptions::BitcoinAddress {
    confirmation_speed: OnchainConfirmationSpeed::Medium,
});
let optional_idempotency_key = Some("<idempotency key uuid>".to_string());
let send_response = sdk
    .send_payment(SendPaymentRequest {
        prepare_response,
        options,
        idempotency_key: optional_idempotency_key,
    })
    .await?;
let payment = send_response.payment;
info!("Payment: {payment:?}");
```



### Spark

In the optional send payment options for Spark addresses, you can set:

- **HTLC Options** - Enables Spark HTLC payments, which are an advanced feature that allows for conditional payments. See the [Spark HTLC Payments](htlcs.md) page for more details and example usage.

```rust
let optional_idempotency_key = Some("<idempotency key uuid>".to_string());
let send_response = sdk
    .send_payment(SendPaymentRequest {
        prepare_response,
        options: None,
        idempotency_key: optional_idempotency_key,
    })
    .await?;
let payment = send_response.payment;
info!("Payment: {payment:?}");
```



### USDC/USDT

Send USDC/USDT has no additional send payment options.

```rust
// Only valid for sends with no token leg (see Retry safety).
let optional_idempotency_key = Some("<idempotency key uuid>".to_string());
let send_response = sdk
    .send_payment(SendPaymentRequest {
        prepare_response,
        options: None,
        idempotency_key: optional_idempotency_key,
    })
    .await?;
let payment = send_response.payment;
info!("Payment: {payment:?}");
```



## Event Flows

Once a send payment is initiated, you can follow and react to the different payment events using the guide below for each payment method. See [listening to events](/guide/events.html) for how to subscribe to events. 

The `SdkEvent::Synced` event is also emitted as the SDK syncs in the background. See [fetching the balance](/llms/rust/guide/get_info.md) for the recommended pattern for refreshing the balance and payments list.

#### Lightning

| Event                | Description                                                                       | UX Suggestion                                    |
| -------------------- | --------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer has been started. Awaiting Lightning payment completion.       | Show payment as pending.                         |
| **PaymentSucceeded** | The Lightning invoice has been paid either over Lightning or via a Spark transfer | Show the payment as complete and call `get_info` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/rust/guide/get_info.md). |
| **PaymentFailed**    | The attempt to pay the Lightning invoice failed.                                  |                                                  |

#### Bitcoin

| Event                | Description                                                                   | UX Suggestion                                    |
| -------------------- | ----------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The Spark transfer has been started. Awaiting on-chain withdrawal completion. | Show payment as pending.                         |
| **PaymentSucceeded** | The payment amount was successfully withdrawn on-chain.                       | Show the payment as complete and call `get_info` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/rust/guide/get_info.md). |

#### Spark

| Event                | Description                     | UX Suggestion                                    |
| -------------------- | ------------------------------- | ------------------------------------------------ |
| **PaymentSucceeded** | The Spark transfer is complete. | Show the payment as complete and call `get_info` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/rust/guide/get_info.md). |

#### USDC/USDT

| Event                | Description                                                                                              | UX Suggestion                                    |
| -------------------- | -------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| **PaymentPending**   | The deposit transfer has been submitted to the provider. The cross-chain leg is awaiting settlement.     | Show payment as pending; the bridge leg may take several minutes depending on the provider and destination chain. |
| **PaymentSucceeded** | The provider reports the cross-chain order terminal. The amount actually delivered to the recipient is carried on the conversion info. | Show the payment as complete and call `get_info` to read the updated balance. The SDK refreshes the cached balance before emitting this event. See [fetching the balance](/llms/rust/guide/get_info.md). |
