# Converting tokens

This process enables funds to be converted either to or from Bitcoin and a selected token. The conversion process takes three steps:

1. [Fetching the limits](#fetching-the-limits) - Get the current limits for converting tokens
2. [Preparing to convert](#preparing-to-convert) - Prepare the conversion by validating the parameters
3. [Convert](#convert)

<h2 id="fetching-the-limits">
    <a class="header" href="#fetching-the-limits">Fetching the limits</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.fetch_convert_token_limits">API docs</a>
</h2>

Before proceeding, you should first verify whether any minimum amount limits apply to sending and receiving tokens during the conversion process. These limits should inform both your guidelines and the input validation logic in your UI.

{{#tabs tokens:fetch-convert-limits}}

<h2 id="preparing-to-convert">
    <a class="header" href="#preparing-to-convert">Preparing to convert</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_convert_token">API docs</a>
</h2>

The next step is to prepare the conversion. This validates that a conversion between Bitcoin and the selected token is possible, that the amount is within the limits of the conversion, confirms that sufficient funds are available, and provides estimates for the received amount and associated fees.

### From token to Bitcoin

When converting from a token, the amount to convert and fees are denominated in token base units. The estimated receive amount is denominated in satoshis.

{{#tabs tokens:prepare-convert-token-to-bitcoin}}

### From Bitcoin to token

When converting from Bitcoin, the amount to convert and fees are denominated in satoshis. The estimated receive amount is denominated in token base units.

{{#tabs tokens:prepare-convert-token-from-bitcoin}}

<h2 id="convert">
    <a class="header" href="#convert">Convert</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.convert_token">API docs</a>
</h2>

Once the conversion has been prepared and the fees are accepted, it can be started by passing:
- **Prepare Response** - The response from the [Preparing to convert](#preparing-to-convert) step.
- **Minimum Slippage** - The optional minimum slippage allowed in basis points. By default a minimum slippage of 50 basis points (0.5%) is set.

**Note:** The price can move between the estimated receive amount and the executed conversion. Setting a minimum slippage ensures the conversion only occurs within that slippage limit.

{{#tabs tokens:convert-token}}
