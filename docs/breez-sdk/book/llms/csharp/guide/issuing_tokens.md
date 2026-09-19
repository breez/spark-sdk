## Issuing tokens

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_token_issuer

The Breez SDK provides a specialized Token Issuer interface for managing custom token issuance on the Spark network using the using the [BTKN protocol](https://docs.spark.money/learn/tokens/hello-btkn). This functionality enables token creators to issue, manage, and control their own tokens with advanced features.

```csharp
var tokenIssuer = sdk.GetTokenIssuer();
```



## Token creation

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.create_issuer_token

Create a custom token with configurable parameters. Define the decimal precision, max supply and if the token can be frozen.

```csharp
var maxSupply = new BigInteger(1000000);
var request = new CreateIssuerTokenRequest(
    name: "My Token",
    ticker: "MTK",
    decimals: 6,
    isFreezable: false,
    maxSupply: maxSupply
);

var tokenMetadata = await tokenIssuer.CreateIssuerToken(request);
Console.WriteLine($"Token identifier: {tokenMetadata.identifier}");
```



### Creating multiple tokens

Token creation is limited to one token per issuer wallet. If you need to create and then manage more than one token using the same mnemonic, we recommend using different account numbers when initializing the SDK.

```csharp
var accountNumber = 21u;

var mnemonic = "<mnemonic words>";
var seed = new Seed.Mnemonic(mnemonic: mnemonic, passphrase: null);
var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
{
    apiKey = "<breez api key>"
};
var builder = new SdkBuilder(config: config, seed: seed);
await builder.WithDefaultStorage(storageDir: "./.data");

// Set the account number for the SDK
await builder.WithAccountNumber(accountNumber);

var sdk = await builder.Build();
```



## Supply Management

### Minting a token

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.mint_issuer_token

Mint to increase the circulating supply of the token.

```csharp
var amount = new BigInteger(1000);
var request = new MintIssuerTokenRequest(
    amount: amount
);
var payment = await tokenIssuer.MintIssuerToken(request);
```



### Burning a token

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.burn_issuer_token

Permanently remove tokens from the circulating supply by burning them.

```csharp
var amount = new BigInteger(1000);
var request = new BurnIssuerTokenRequest(
    amount: amount
);
var payment = await tokenIssuer.BurnIssuerToken(request);
```



### Listing mint or burn payments

Mint or burn payments are included in the regular payment history that is obtained when [Listing payments](./list_payments.md).

You can filter by token transaction type to only include mint, burn or transfer payments. Transfer payments are regular token payments that are not mint or burn payments.

```csharp
// Provide one or multiple of the following filters to 
// the `paymentDetailsFilter` field when listing payments
var paymentDefailsTransferFilter = new PaymentDetailsFilter.Token(
    txType: TokenTransactionType.Transfer,
    txHash: null,
    conversionRefundNeeded: null
);
var paymentDefailsMintFilter = new PaymentDetailsFilter.Token(
    txType: TokenTransactionType.Mint,
    txHash: null,
    conversionRefundNeeded: null
);
var paymentDefailsBurnFilter = new PaymentDetailsFilter.Token(
    txType: TokenTransactionType.Burn,
    txHash: null,
    conversionRefundNeeded: null
);
```



## Query balance & metadata

Retrieve the current issued token balance and fetch the token metadata.

```csharp
var tokenBalance = await tokenIssuer.GetIssuerTokenBalance();
Console.WriteLine($"Token balance: {tokenBalance.balance}");

var tokenMetadata = await tokenIssuer.GetIssuerTokenMetadata();
Console.WriteLine($"Token ticker: {tokenMetadata.ticker}");
```



## Freeze and unfreeze tokens

Freeze and unfreeze tokens at a specific Spark address if the token metadata allows it.

```csharp
var sparkAddress = "<spark address>";
var freezeRequest = new FreezeIssuerTokenRequest(
    address: sparkAddress
);
var freezeReponse = await tokenIssuer.FreezeIssuerToken(freezeRequest);

var unfreezeRequest = new UnfreezeIssuerTokenRequest(
    address: sparkAddress
);
var unfreezeResponse = await tokenIssuer.UnfreezeIssuerToken(unfreezeRequest);
```
