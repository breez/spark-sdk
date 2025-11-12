<h2 id="issuing-tokens">
    <a class="header" href="#issuing-tokens">Issuing tokens</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_token_issuer">API docs</a>
</h2>

The Breez SDK provides a specialized Token Issuer interface for managing custom token issuance on the Spark network using the using the [BTKN protocol](https://docs.spark.money/learn/tokens/hello-btkn). This functionality enables token creators to issue, manage, and control their own tokens with advanced features.

{{#tabs issuing_tokens:get-issuer-sdk}}

<h2 id="token-creation">
    <a class="header" href="#token-creation">Token creation</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.create_issuer_token">API docs</a>
</h2>

Create a custom token with configurable parameters. Define the decimal precision, max supply and if the token can be frozen.

**Note:** Token creation is limited to one token per issuer wallet

{{#tabs issuing_tokens:create-token}}

## Supply Management

<h3 id="minting-a-token">
    <a class="header" href="#minting-a-token">Minting a token</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.mint_issuer_token">API docs</a>
</h3>

Mint to increase the circulating supply of the token.

{{#tabs issuing_tokens:mint-token}}

<h3 id="burning-a-token">
    <a class="header" href="#burning-a-token">Burning a token</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.burn_issuer_token">API docs</a>
</h3>

Permanently remove tokens from the circulating supply by burning them.

{{#tabs issuing_tokens:burn-token}}

## Query balance & metadata

Retrieve the current issue token balance and fetch the token metadata.

{{#tabs issuing_tokens:get-token-metadata}}

## Freeze and unfreeze tokens

Freeze and unfreeze tokens at a specific Spark address if the token metadata allows it.

{{#tabs issuing_tokens:freeze-token}}
