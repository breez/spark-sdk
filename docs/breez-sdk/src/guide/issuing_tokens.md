## Issuing tokens

The Breez SDK provides a specialized Token Issuer interface for managing custom token issuance on the Spark network using the using the [BTKN protocol](https://docs.spark.money/learn/tokens/hello-btkn). This functionality enables token creators to issue, manage, and control their own tokens with advanced features.

{{#tabs issuing_tokens:get-token-issuer}}

## Token creation

Create a custom token with configurable parameters. Define the decimal precision, max supply and if the token can be frozen.

{{#tabs issuing_tokens:create-token}}

### Attaching extra metadata

You can attach up to 1024 bytes of your own data to the token with {{#name extra_metadata}}, such as an icon URL or a JSON description. It is part of the token identifier, so it cannot be changed after the token is created.

Wallets read it from the {{#name extra_metadata}} field of the token metadata returned by {{#name get_tokens_metadata}} and {{#name get_issuer_token_metadata}}, and of the token balances returned by {{#name get_info}}. It is always unset in the token metadata of payment details.

### Creating multiple tokens

Token creation is limited to one token per issuer wallet. If you need to create and then manage more than one token using the same mnemonic, we recommend using different account numbers when initializing the SDK.

{{#tabs issuing_tokens:custom-account-number}}

## Supply Management

### Minting a token

Mint to increase the circulating supply of the token.

{{#tabs issuing_tokens:mint-token}}

### Burning a token

Permanently remove tokens from the circulating supply by burning them.

{{#tabs issuing_tokens:burn-token}}

### Listing mint or burn payments

Mint or burn payments are included in the regular payment history that is obtained when [Listing payments](./list_payments.md).

You can filter by token transaction type to only include mint, burn or transfer payments. Transfer payments are regular token payments that are not mint or burn payments.

{{#tabs issuing_tokens:list-mint-burn-payments}}

## Query balance & metadata

Retrieve the current issued token balance and fetch the token metadata.

{{#tabs issuing_tokens:get-token-metadata}}

## Freeze and unfreeze tokens

Freeze and unfreeze tokens at a specific Spark address if the token metadata allows it.

{{#tabs issuing_tokens:freeze-token}}
