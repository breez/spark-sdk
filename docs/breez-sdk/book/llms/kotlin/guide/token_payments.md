# Sending and receiving tokens

Spark supports tokens using the [BTKN protocol](https://docs.spark.money/learn/tokens/hello-btkn). The Breez SDK enables you to send and receive these tokens using the standard payments API.

## Fetching token balances

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_info

Token balances for all tokens currently held in the wallet can be retrieved along with general wallet information. Each token balance includes both the balance amount and the token metadata (identifier, name, ticker, issuer public key, etc.).

```kotlin
try {
    // ensureSynced: true will ensure the SDK is synced with the Spark network
    // before returning the balance
    val info = sdk.getInfo(GetInfoRequest(false))

    // Token balances are a map of token identifier to balance
    val tokenBalances = info.tokenBalances
    for ((tokenId, tokenBalance) in tokenBalances) {
        println("Token ID: $tokenId")
        println("Balance: ${tokenBalance.balance}")
        println("Name: ${tokenBalance.tokenMetadata.name}")
        println("Ticker: ${tokenBalance.tokenMetadata.ticker}")
        println("Decimals: ${tokenBalance.tokenMetadata.decimals}")
    }
} catch (e: Exception) {
    // handle error
}
```



**Developer note**

Token balances are cached for fast responses. For details on ensuring up-to-date balances, see the <a href="./get_info.md#fetching-the-balance">Fetching the balance</a> section.

## Fetching token metadata

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_tokens_metadata

Token metadata can be fetched for specific tokens by providing their identifiers. This is especially useful for retrieving metadata for tokens that are not currently held in the wallet. The metadata is cached locally after the first fetch for faster subsequent lookups.

```kotlin
try {
    val response = 
        sdk.getTokensMetadata(
            GetTokensMetadataRequest(
                tokenIdentifiers = listOf("<token identifier 1>", "<token identifier 2>")
        )
    )   

    val tokensMetadata = response.tokensMetadata
    for (tokenMetadata in tokensMetadata) {
        println("Token ID: ${tokenMetadata.identifier}")
        println("Name: ${tokenMetadata.name}")
        println("Ticker: ${tokenMetadata.ticker}")
        println("Decimals: ${tokenMetadata.decimals}")
        println("Max Supply: ${tokenMetadata.maxSupply}")
        println("Is Freezable: ${tokenMetadata.isFreezable}")
    }
} catch (e: Exception) {
    // handle error
}
```



## Receiving a token payment

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

Token payments can be received using either a Spark address or invoice. Using an invoice is useful to impose restrictions on the payment, such as the token to receive, amount, expiry, and who can pay it.

### Spark address

Token payments use the same Spark address as Bitcoin payments - no separate address is required. Your application can retrieve the Spark address as described in the [Receiving a payment](./receive_payment.md#spark) guide. The payer will use this address to send tokens to the wallet.

### Spark invoice

Spark token invoices can be created using the same API as Bitcoin Spark invoices. The only difference is that a token identifier is provided.

```kotlin
try {
    val tokenIdentifier = "<token identifier>"
    val optionalDescription = "<invoice description>"
    // Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer, which is included in
    // package)
    val optionalAmount = BigInteger.fromLong(5_000L)
    // Android (BigInteger from java.math)
    // val optionalAmount = BigInteger.valueOf(5_000L)
    // Optionally set the expiry UNIX timestamp in seconds
    val optionalExpiryTimeSeconds = 1716691200.toULong()
    val optionalSenderPublicKey = "<sender public key>"

    val request = ReceivePaymentRequest(
        ReceivePaymentMethod.SparkInvoice(
            tokenIdentifier = tokenIdentifier,
            description = optionalDescription,
            amount = optionalAmount,
            expiryTime = optionalExpiryTimeSeconds,
            senderPublicKey = optionalSenderPublicKey
        )
    )
    val response = sdk.receivePayment(request)

    val paymentRequest = response.paymentRequest
    println("Payment request: $paymentRequest")
    val receiveFee = response.fee
    println("Fees: $receiveFee token base units")
} catch (e: Exception) {
    // handle error
}
```



## Sending a token payment

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_send_payment

To send tokens, provide a Spark address as the payment request. The token identifier must be specified in one of two ways:

1. **Using a Spark invoice**: If the payee provides a Spark address with an embedded token identifier and amount (a Spark invoice), the SDK automatically extracts and uses those values.
2. **Manual specification**: For a plain Spark address without embedded payment details, your application must provide both the token identifier and amount parameters when preparing the payment.

Your application can use the [parse](./parse.md) functionality to determine if a Spark address contains embedded token payment details before preparing the payment.

The code example below demonstrates manual specification. Follow the standard prepare/send payment flow as described in the [Sending a payment](./send_payment.md) guide.

**Developer note**

Payments can be sent without holding an asset by converting on-the-fly as a step before sending a payment. See <a href="./token_conversion.md">Converting tokens</a> for more information.

```kotlin
try {
    val paymentRequest = "<spark address or invoice>"
    // Token identifier must match the invoice in case it specifies one.
    val tokenIdentifier = "<token identifier>"
    // Set the amount of tokens you wish to send (in token base units).
    // Kotlin MPP (BigInteger from com.ionspin.kotlin.bignum.integer)
    val amount = BigInteger.fromLong(1_000L)
    // Android (BigInteger from java.math)
    // val amount = BigInteger.valueOf(1_000L)

    val prepareResponse =
        sdk.prepareSendPayment(
            PrepareSendPaymentRequest(
                paymentRequest = PaymentRequest.Input(input = paymentRequest),
                amount = amount,
                tokenIdentifier = tokenIdentifier,
                conversionOptions = null,
                feePolicy = null,
            )
        )

    // If the fees are acceptable, continue to send the token payment
    when (val method = prepareResponse.paymentMethod) {
        is SendPaymentMethod.SparkAddress -> {
            println("Token ID: ${method.tokenIdentifier}")
            println("Fees: ${method.fee} token base units")
        }
        is SendPaymentMethod.SparkInvoice -> {
            println("Token ID: ${method.tokenIdentifier}")
            println("Fees: ${method.fee} token base units")
        }
        else -> {}
    }

    // Send the token payment
    val sendResponse =
        sdk.sendPayment(
            SendPaymentRequest(prepareResponse = prepareResponse, options = null)
        )
    val payment = sendResponse.payment
    println("Payment: $payment")
} catch (e: Exception) {
    // handle error
}
```



To pay several recipients at once, see [Sending to multiple recipients](./batch_send.md): one transaction can pay multiple payees, across several tokens, mixing Spark addresses and invoices. A batch that pays an invoice stays on one token.

## Listing token payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

Token payments are included in the regular payment history alongside Bitcoin payments. Your application can retrieve and distinguish token payments from other payment types using the standard payment listing functionality. See the [Listing payments](./list_payments.md) guide for more details.
