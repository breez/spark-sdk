## Issuing tokens

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_token_issuer

The Breez SDK provides a specialized Token Issuer interface for managing custom token issuance on the Spark network using the using the [BTKN protocol](https://docs.spark.money/learn/tokens/hello-btkn). This functionality enables token creators to issue, manage, and control their own tokens with advanced features.

```rust
let token_issuer = sdk.get_token_issuer();
```



## Token creation

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.create_issuer_token

Create a custom token with configurable parameters. Define the decimal precision, max supply and if the token can be frozen.

```rust
let request = CreateIssuerTokenRequest {
    name: "My Token".to_string(),
    ticker: "MTK".to_string(),
    decimals: 6,
    is_freezable: false,
    max_supply: 1_000_000,
};
let token_metadata = token_issuer.create_issuer_token(request).await?;
info!("Token identifier: {}", token_metadata.identifier);
```



### Creating multiple tokens

Token creation is limited to one token per issuer wallet. If you need to create and then manage more than one token using the same mnemonic, we recommend using different account numbers when initializing the SDK.

```rust
let account_number = 21;

let mnemonic = "<mnemonic words>".to_string();
let seed = Seed::Mnemonic {
    mnemonic,
    passphrase: None,
};
let config = default_config(Network::Mainnet);
let mut builder = SdkBuilder::new(config, seed);
builder = builder.with_default_storage("./.data".to_string());

// Set the account number for the SDK
builder = builder.with_account_number(account_number);

let sdk = builder.build().await?;
```



## Supply Management

### Minting a token

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.mint_issuer_token

Mint to increase the circulating supply of the token.

```rust
let request = MintIssuerTokenRequest { amount: 1_000 };

let payment = token_issuer.mint_issuer_token(request).await?;
```



### Burning a token

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.burn_issuer_token

Permanently remove tokens from the circulating supply by burning them.

```rust
let request = BurnIssuerTokenRequest { amount: 1_000 };

let payment = token_issuer.burn_issuer_token(request).await?;
```



### Listing mint or burn payments

Mint or burn payments are included in the regular payment history that is obtained when [Listing payments](./list_payments.md).

You can filter by token transaction type to only include mint, burn or transfer payments. Transfer payments are regular token payments that are not mint or burn payments.

```rust
// Provide one or multiple of the following filters to
// the `payment_details_filter` field when listing payments
let payment_details_transfer_filter = PaymentDetailsFilter::Token {
    tx_type: Some(TokenTransactionType::Transfer),
    tx_hash: None,
    conversion_refund_needed: None,
};
let payment_details_mint_filter = PaymentDetailsFilter::Token {
    tx_type: Some(TokenTransactionType::Mint),
    tx_hash: None,
    conversion_refund_needed: None,
};
let payment_details_burn_filter = PaymentDetailsFilter::Token {
    tx_type: Some(TokenTransactionType::Burn),
    tx_hash: None,
    conversion_refund_needed: None,
};
```



## Query balance & metadata

Retrieve the current issued token balance and fetch the token metadata.

```rust
let token_balance = token_issuer.get_issuer_token_balance().await?;
info!("Token balance: {}", token_balance.balance);

let token_metadata = token_issuer.get_issuer_token_metadata().await?;
info!("Token ticker: {}", token_metadata.ticker);
```



## Freeze and unfreeze tokens

Freeze and unfreeze tokens at a specific Spark address if the token metadata allows it.

```rust
let spark_address = "<spark address>".to_string();
// Freeze the tokens held at the specified Spark address
let freeze_request = FreezeIssuerTokenRequest {
    address: spark_address.clone(),
};
let freeze_response = token_issuer.freeze_issuer_token(freeze_request).await?;

// Unfreeze the tokens held at the specified Spark address
let unfreeze_request = UnfreezeIssuerTokenRequest {
    address: spark_address,
};
let unfreeze_response = token_issuer.unfreeze_issuer_token(unfreeze_request).await?;
```
