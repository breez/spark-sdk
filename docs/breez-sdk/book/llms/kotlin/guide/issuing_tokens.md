## Issuing tokens

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_token_issuer

The Breez SDK provides a specialized Token Issuer interface for managing custom token issuance on the Spark network using the using the [BTKN protocol](https://docs.spark.money/learn/tokens/hello-btkn). This functionality enables token creators to issue, manage, and control their own tokens with advanced features.

```kotlin
val tokenIssuer = sdk.getTokenIssuer()
```



## Token creation

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.create_issuer_token

Create a custom token with configurable parameters. Define the decimal precision, max supply and if the token can be frozen.

```kotlin
try {
    val request =
            CreateIssuerTokenRequest(
                    name = "My Token",
                    ticker = "MTK",
                    decimals = 6.toUInt(),
                    isFreezable = false,
                    maxSupply = BigInteger.fromLong(1_000_000L)
            )
    val tokenMetadata = tokenIssuer.createIssuerToken(request)
    // Log.v("Breez", "Token identifier: ${tokenMetadata.identifier}")
} catch (e: Exception) {
    // Handle exception
}
```



### Creating multiple tokens

Token creation is limited to one token per issuer wallet. If you need to create and then manage more than one token using the same mnemonic, we recommend using different account numbers when initializing the SDK.

```kotlin
val accountNumber = 21u

val mnemonic = "<mnemonic words>"
val seed = Seed.Mnemonic(mnemonic, null)
val config = defaultConfig(Network.MAINNET)
config.apiKey = "<breez api key>"

try {
    val builder = SdkBuilder(config, seed)
    builder.withDefaultStorage("./.data")

    // Set the account number for the SDK
    builder.withAccountNumber(accountNumber)

    val sdk = builder.build()
} catch (e: Exception) {
    // handle error
}
```



## Supply Management

### Minting a token

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.mint_issuer_token

Mint to increase the circulating supply of the token.

```kotlin
try {
    val request =
            MintIssuerTokenRequest(
                    amount = BigInteger.fromLong(1_000L),
            )
    val payment = tokenIssuer.mintIssuerToken(request)
} catch (e: Exception) {
    // Handle exception
}
```



### Burning a token

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.burn_issuer_token

Permanently remove tokens from the circulating supply by burning them.

```kotlin
try {
    val request =
            BurnIssuerTokenRequest(
                    amount = BigInteger.fromLong(1_000L),
            )
    val payment = tokenIssuer.burnIssuerToken(request)
} catch (e: Exception) {
    // Handle exception
}
```



### Listing mint or burn payments

Mint or burn payments are included in the regular payment history that is obtained when [Listing payments](./list_payments.md).

You can filter by token transaction type to only include mint, burn or transfer payments. Transfer payments are regular token payments that are not mint or burn payments.

```kotlin
// Provide one or multiple of the following filters to
// the `paymentDetailsFilter` field when listing payments
val paymentDetailsTransferFilter =
        PaymentDetailsFilter.Token(
                txType = TokenTransactionType.TRANSFER,
                txHash = null,
                conversionRefundNeeded = null
        )
val paymentDetailsMintFilter =
        PaymentDetailsFilter.Token(
                txType = TokenTransactionType.MINT,
                txHash = null,
                conversionRefundNeeded = null
        )
val paymentDetailsBurnFilter =
        PaymentDetailsFilter.Token(
                txType = TokenTransactionType.BURN,
                txHash = null,
                conversionRefundNeeded = null
        )
```



## Query balance & metadata

Retrieve the current issued token balance and fetch the token metadata.

```kotlin
try {
    val tokenBalance = tokenIssuer.getIssuerTokenBalance()
    // Log.v("Breez", "Token balance: ${tokenBalance.balance}")

    val tokenMetadata = tokenIssuer.getIssuerTokenMetadata()
    // Log.v("Breez", "Token ticker: ${tokenMetadata.ticker}")
} catch (e: Exception) {
    // Handle exception
}
```



## Freeze and unfreeze tokens

Freeze and unfreeze tokens at a specific Spark address if the token metadata allows it.

```kotlin
try {
    val sparkAddress = "<spark address>"
    // Freeze the tokens held at the specified Spark address
    val freezeRequest =
            FreezeIssuerTokenRequest(
                    address = sparkAddress,
            )
    val freezeResponse = tokenIssuer.freezeIssuerToken(freezeRequest)

    // Unfreeze the tokens held at the specified Spark address
    val unfreezeRequest =
            UnfreezeIssuerTokenRequest(
                    address = sparkAddress,
            )
    val unfreezeResponse = tokenIssuer.unfreezeIssuerToken(unfreezeRequest)
} catch (e: Exception) {
    // Handle exception
}
```
