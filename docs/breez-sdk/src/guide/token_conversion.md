# Converting tokens

Token conversion enables payments to be made without holding the required asset by converting on-the-fly between Bitcoin and tokens using the Flashnet protocol.

<h2 id="fetching-conversion-limits">
    <a class="header" href="#fetching-conversion-limits">Fetching conversion limits</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.fetch_conversion_limits">API docs</a>
</h2>

Before performing a conversion, you can fetch the minimum amounts required for the conversion. The limits depend on the conversion direction:

- **Bitcoin to token**: Minimum Bitcoin amount (in satoshis) and minimum token amount to receive (in token base units)
- **Token to Bitcoin**: Minimum token amount (in token base units) and minimum Bitcoin amount to receive (in satoshis)

{{#tabs tokens:fetch-conversion-limits}}

<div class="warning">
<h4>Developer note</h4>
Amounts are denominated in satoshis for Bitcoin (1 BTC = 100,000,000 sats) and in token base units for tokens. Token base units depend on the token's decimal specification.
</div>

<h2 id="bitcoin-to-token">
    <a class="header" href="#bitcoin-to-token">Converting Bitcoin to tokens</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment">API docs</a>
</h2>

Token conversion enables payments of tokens like <a href="https://sparkscan.io/token/3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca?network=mainnet" target="_blank">USDB</a> to be made without holding the token, but instead using Bitcoin.

To do so, when preparing to send a payment, set the conversion options. The conversion will first calculate the Bitcoin amount needed to be converted into the token, convert Bitcoin into that token amount, and then finally complete the payment.

{{#tabs tokens:prepare-send-payment-with-conversion}}

<div class="warning">
<h4>Developer note</h4>
When a conversion fails due to exceeding the maximum slippage, the conversion will be refunded automatically.
</div>

<div class="warning">
<h4>Developer note</h4>
The conversion may result in some token balance remaining in the wallet after the payment is sent. This remaining balance is to account for slippage in the conversion.
</div>

<h2 id="token-to-bitcoin">
    <a class="header" href="#token-to-bitcoin">Converting tokens to Bitcoin</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment">API docs</a>
</h2>

Token conversion also enables Bitcoin payments to be made without holding the required Bitcoin, but instead using a supported token asset like <a href="https://sparkscan.io/token/3206c93b24a4d18ea19d0a9a213204af2c7e74a6d16c7535cc5d33eca4ad1eca?network=mainnet" target="_blank">USDB</a>.

To do so, when preparing to send a payment, set the conversion options. The conversion will first calculate the amount needed to be converted into Bitcoin, convert the token into that Bitcoin amount, and then finally complete the payment.

{{#tabs send_payment:prepare-send-payment-with-conversion}}

<div class="warning">
<h4>Developer note</h4>
When a conversion fails due to exceeding the maximum slippage, the conversion will be refunded automatically.
</div>

<div class="warning">
<h4>Developer note</h4>
The conversion may result in some Bitcoin remaining in the wallet after the payment is sent. This remaining Bitcoin is to account for slippage in the conversion.
</div>
