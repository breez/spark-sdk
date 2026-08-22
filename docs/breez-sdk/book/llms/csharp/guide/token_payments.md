# Sending and receiving tokens

Spark supports tokens using the [BTKN protocol](https://docs.spark.money/learn/tokens/hello-btkn). The Breez SDK enables you to send and receive these tokens using the standard payments API.

## Fetching token balances

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_info

Token balances for all tokens currently held in the wallet can be retrieved along with general wallet information. Each token balance includes both the balance amount and the token metadata (identifier, name, ticker, issuer public key, etc.).

```csharp
// ensureSynced: true will ensure the SDK is synced with the Spark network
// before returning the balance
var info = await sdk.GetInfo(request: new GetInfoRequest(ensureSynced: false));

// Token balances are a map of token identifier to balance
var tokenBalances = info.tokenBalances;
foreach (var kvp in tokenBalances)
{
    var tokenId = kvp.Key;
    var tokenBalance = kvp.Value;
    Console.WriteLine($"Token ID: {tokenId}");
    Console.WriteLine($"Balance: {tokenBalance.balance}");
    Console.WriteLine($"Name: {tokenBalance.tokenMetadata.name}");
    Console.WriteLine($"Ticker: {tokenBalance.tokenMetadata.ticker}");
    Console.WriteLine($"Decimals: {tokenBalance.tokenMetadata.decimals}");
}
```



**Developer note**

Token balances are cached for fast responses. For details on ensuring up-to-date balances, see the <a href="./get_info.md#fetching-the-balance">Fetching the balance</a> section.

## Fetching token metadata

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_tokens_metadata

Token metadata can be fetched for specific tokens by providing their identifiers. This is especially useful for retrieving metadata for tokens that are not currently held in the wallet. The metadata is cached locally after the first fetch for faster subsequent lookups.

```csharp
var response = await sdk.GetTokensMetadata(
    request: new GetTokensMetadataRequest(
        tokenIdentifiers: new string[] { "<token identifier 1>", "<token identifier 2>" }
    )
);

var tokensMetadata = response.tokensMetadata;
foreach (var tokenMetadata in tokensMetadata)
{
    Console.WriteLine($"Token ID: {tokenMetadata.identifier}");
    Console.WriteLine($"Name: {tokenMetadata.name}");
    Console.WriteLine($"Ticker: {tokenMetadata.ticker}");
    Console.WriteLine($"Decimals: {tokenMetadata.decimals}");
    Console.WriteLine($"Max Supply: {tokenMetadata.maxSupply}");
    Console.WriteLine($"Is Freezable: {tokenMetadata.isFreezable}");
}
```



## Receiving a token payment

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.receive_payment

Token payments can be received using either a Spark address or invoice. Using an invoice is useful to impose restrictions on the payment, such as the token to receive, amount, expiry, and who can pay it.

### Spark address

Token payments use the same Spark address as Bitcoin payments - no separate address is required. Your application can retrieve the Spark address as described in the [Receiving a payment](./receive_payment.md#spark) guide. The payer will use this address to send tokens to the wallet.

### Spark invoice

Spark token invoices can be created using the same API as Bitcoin Spark invoices. The only difference is that a token identifier is provided.

```csharp
var tokenIdentifier = "<token identifier>";
var optionalDescription = "<invoice description>";
var optionalAmount = new BigInteger(5000);
// Optionally set the expiry UNIX timestamp in seconds
var optionalExpiryTimeSeconds = 1716691200UL;
var optionalSenderPublicKey = "<sender public key>";

var request = new ReceivePaymentRequest(
    paymentMethod: new ReceivePaymentMethod.SparkInvoice(
        tokenIdentifier: tokenIdentifier,
        description: optionalDescription,
        amount: optionalAmount,
        expiryTime: optionalExpiryTimeSeconds,
        senderPublicKey: optionalSenderPublicKey
    )
);
var response = await sdk.ReceivePayment(request: request);

var paymentRequest = response.paymentRequest;
Console.WriteLine($"Payment request: {paymentRequest}");
var receiveFee = response.fee;
Console.WriteLine($"Fees: {receiveFee} token base units");
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

```csharp
var paymentRequest = "<spark address or invoice>";
// Token identifier must match the invoice in case it specifies one.
var tokenIdentifier = "<token identifier>";
// Set the amount of tokens you wish to send.
ulong? amount = 1_000UL;

var prepareResponse = await sdk.PrepareSendPayment(
    request: new PrepareSendPaymentRequest(
        paymentRequest: new PaymentRequest.Input(input: paymentRequest),
        amount: amount,
        tokenIdentifier: tokenIdentifier,
        conversionOptions: null,
        feePolicy: null
    )
);

// If the fees are acceptable, continue to send the token payment
if (prepareResponse.paymentMethod is SendPaymentMethod.SparkAddress sparkAddress)
{
    Console.WriteLine($"Token ID: {sparkAddress.tokenIdentifier}");
    Console.WriteLine($"Fees: {sparkAddress.fee} token base units");
}
if (prepareResponse.paymentMethod is SendPaymentMethod.SparkInvoice sparkInvoice)
{
    Console.WriteLine($"Token ID: {sparkInvoice.tokenIdentifier}");
    Console.WriteLine($"Fees: {sparkInvoice.fee} token base units");
}

// Send the token payment
var sendResponse = await sdk.SendPayment(
    request: new SendPaymentRequest(
        prepareResponse: prepareResponse,
        options: null
    )
);
var payment = sendResponse.payment;
Console.WriteLine($"Payment: {payment}");
```



To pay several recipients at once, see [Sending to multiple recipients](./batch_send.md): one transaction can pay multiple payees, across several tokens, mixing Spark addresses and invoices. A batch that pays an invoice stays on one token.

## Listing token payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

Token payments are included in the regular payment history alongside Bitcoin payments. Your application can retrieve and distinguish token payments from other payment types using the standard payment listing functionality. See the [Listing payments](./list_payments.md) guide for more details.
