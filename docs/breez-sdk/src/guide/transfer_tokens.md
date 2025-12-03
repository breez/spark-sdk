# Transfer tokens

This process enables funds to be transferred either to or from Bitcoin and a selected token. The transfer process takes two steps:

1. [Preparing the transfer](#preparing-transfers)
1. [Transfer](#transfer)

<h2 id="preparing-transfers">
    <a class="header" href="#preparing-transfers">Preparing the transfer</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_transfer_token">API docs</a>
</h2>

The first step is to prepare the transfer. This step validates that a transfer between Bitcoin and the selected token is possible, that the amount is within the limits of the transfer, confirms that sufficient funds are available, and provides estimates for the received amount and associated fees.

### From token to Bitcoin

When transferring from a token the amount to transfer and fees are denominated in token base units. The estimated receive amount is denominated in satoshis.

{{#tabs tokens:prepare-transfer-token-to-bitcoin}}

### From Bitcoin to token

When transferring from Bitcoin the amount to transfer and fees are denominated in satoshis. The estimated receive amount is denominated in token base units.

{{#tabs tokens:prepare-transfer-token-from-bitcoin}}

<h2 id="transfer">
    <a class="header" href="#transfer">Transfer</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.transfer_token">API docs</a>
</h2>

Once the transfer has been prepared and the fees are accepted, the transfer can be started by passing:
- **Prepare Response** - The response from the [Preparing the Transfer](#preparing-transfer) step.
- **Minimum Slippage** - The optional minimum slippage allowed in basis points. By default a minimum slippage of 50 basis points (0.5%) is set.

**Note:** The price can move between the estimated receive amount and the executed transfer. Setting a minimum slippage ensures the transfer only occurs within that slippage limit.

{{#tabs tokens:transfer-token}}
