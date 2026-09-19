## Issuing tokens

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_token_issuer

The Breez SDK provides a specialized Token Issuer interface for managing custom token issuance on the Spark network using the using the [BTKN protocol](https://docs.spark.money/learn/tokens/hello-btkn). This functionality enables token creators to issue, manage, and control their own tokens with advanced features.

```typescript
const tokenIssuer = sdk.getTokenIssuer()
```



## Token creation

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.create_issuer_token

Create a custom token with configurable parameters. Define the decimal precision, max supply and if the token can be frozen.

```typescript
const tokenMetadata = await tokenIssuer.createIssuerToken({
  name: 'My Token',
  ticker: 'MTK',
  decimals: 6,
  isFreezable: false,
  maxSupply: BigInt(1_000_000)
})
console.debug(`Token identifier: ${tokenMetadata.identifier}`)
```



### Creating multiple tokens

Token creation is limited to one token per issuer wallet. If you need to create and then manage more than one token using the same mnemonic, we recommend using different account numbers when initializing the SDK.

```typescript
await init()

const accountNumber = 21

const mnemonic = '<mnemonic words>'
const seed: Seed = { type: 'mnemonic', mnemonic, passphrase: undefined }
const config = defaultConfig('mainnet')
config.apiKey = '<breez api key>'
let builder = SdkBuilder.new(config, seed)
builder = await builder.withDefaultStorage('./.data')

// Set the account number for the SDK
builder = builder.withAccountNumber(accountNumber)

const sdk = await builder.build()
```



## Supply Management

### Minting a token

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.mint_issuer_token

Mint to increase the circulating supply of the token.

```typescript
const payment = await tokenIssuer.mintIssuerToken({
  amount: BigInt(1_000)
})
```



### Burning a token

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.burn_issuer_token

Permanently remove tokens from the circulating supply by burning them.

```typescript
const payment = await tokenIssuer.burnIssuerToken({
  amount: BigInt(1_000)
})
```



### Listing mint or burn payments

Mint or burn payments are included in the regular payment history that is obtained when [Listing payments](./list_payments.md).

You can filter by token transaction type to only include mint, burn or transfer payments. Transfer payments are regular token payments that are not mint or burn payments.

```typescript
// Provide one or multiple of the following filters to
// the `paymentDetailsFilter` field when listing payments
const paymentDetailsTransferFilter = {
  type: 'token',
  txType: 'transfer'
}
const paymentDetailsMintFilter = {
  type: 'token',
  txType: 'mint'
}
const paymentDetailsBurnFilter = {
  type: 'token',
  txType: 'burn'
}
```



## Query balance & metadata

Retrieve the current issued token balance and fetch the token metadata.

```typescript
const tokenBalance = await tokenIssuer.getIssuerTokenBalance()
console.debug(`Token balance: ${tokenBalance.balance}`)

const tokenMetadata = await tokenIssuer.getIssuerTokenMetadata()
console.debug(`Token ticker: ${tokenMetadata.ticker}`)
```



## Freeze and unfreeze tokens

Freeze and unfreeze tokens at a specific Spark address if the token metadata allows it.

```typescript
const sparkAddress = '<spark address>'
// Freeze the tokens held at the specified Spark address
const freezeResponse = await tokenIssuer.freezeIssuerToken({
  address: sparkAddress
})

// Unfreeze the tokens held at the specified Spark address
const unfreezeResponse = await tokenIssuer.unfreezeIssuerToken({
  address: sparkAddress
})
```
