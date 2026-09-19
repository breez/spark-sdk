## Issuing tokens

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_token_issuer

The Breez SDK provides a specialized Token Issuer interface for managing custom token issuance on the Spark network using the using the [BTKN protocol](https://docs.spark.money/learn/tokens/hello-btkn). This functionality enables token creators to issue, manage, and control their own tokens with advanced features.

```swift
let tokenIssuer = sdk.getTokenIssuer()
```



## Token creation

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.create_issuer_token

Create a custom token with configurable parameters. Define the decimal precision, max supply and if the token can be frozen.

```swift
let request = CreateIssuerTokenRequest(
    name: "My Token",
    ticker: "MTK",
    decimals: UInt32(6),
    isFreezable: false,
    maxSupply: BInt(1_000_000)
)
let tokenMetadata = try await tokenIssuer.createIssuerToken(request: request)
print("Token identifier: {}", tokenMetadata.identifier)
```



### Creating multiple tokens

Token creation is limited to one token per issuer wallet. If you need to create and then manage more than one token using the same mnemonic, we recommend using different account numbers when initializing the SDK.

```swift
let accountNumber = UInt32(21)

let mnemonic = "<mnemonic words>"
let seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: nil)
let config = defaultConfig(network: Network.mainnet)
let builder = SdkBuilder(config: config, seed: seed)
await builder.withDefaultStorage(storageDir: "./.data")

// Set the account number for the SDK
await builder.withAccountNumber(accountNumber: accountNumber)

let sdk = try await builder.build()
```



## Supply Management

### Minting a token

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.mint_issuer_token

Mint to increase the circulating supply of the token.

```swift
let request = MintIssuerTokenRequest(
    amount: BInt(1_000)
)
let payment = try await tokenIssuer.mintIssuerToken(request: request)
```



### Burning a token

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.burn_issuer_token

Permanently remove tokens from the circulating supply by burning them.

```swift
let request = BurnIssuerTokenRequest(
    amount: BInt(1_000)
)
let payment = try await tokenIssuer.burnIssuerToken(request: request)
```



### Listing mint or burn payments

Mint or burn payments are included in the regular payment history that is obtained when [Listing payments](./list_payments.md).

You can filter by token transaction type to only include mint, burn or transfer payments. Transfer payments are regular token payments that are not mint or burn payments.

```swift
// Provide one or multiple of the following filters to
// the `paymentDetailsFilter` field when listing payments
let paymentDetailsTransferFilter = PaymentDetailsFilter.token(
    conversionRefundNeeded: nil, txHash: nil,
    txType: TokenTransactionType.transfer)
let paymentDetailsMintFilter = PaymentDetailsFilter.token(
    conversionRefundNeeded: nil, txHash: nil,
    txType: TokenTransactionType.mint)
let paymentDetailsBurnFilter = PaymentDetailsFilter.token(
    conversionRefundNeeded: nil, txHash: nil,
    txType: TokenTransactionType.burn)
```



## Query balance & metadata

Retrieve the current issued token balance and fetch the token metadata.

```swift
let tokenBalance = try await tokenIssuer.getIssuerTokenBalance()
print("Token balance: {}", tokenBalance.balance)

let tokenMetadata = try await tokenIssuer.getIssuerTokenMetadata()
print("Token ticker: {}", tokenMetadata.ticker)
```



## Freeze and unfreeze tokens

Freeze and unfreeze tokens at a specific Spark address if the token metadata allows it.

```swift
let sparkAddress = "<spark address>"
// Freeze the tokens held at the specified Spark address
let freezeRequest = FreezeIssuerTokenRequest(
    address: sparkAddress
)
let freezeResponse = try await tokenIssuer.freezeIssuerToken(request: freezeRequest)

// Unfreeze the tokens held at the specified Spark address
let unfreezeRequest = UnfreezeIssuerTokenRequest(
    address: sparkAddress
)
let unfreezeResponse = try await tokenIssuer.unfreezeIssuerToken(request: unfreezeRequest)
```
