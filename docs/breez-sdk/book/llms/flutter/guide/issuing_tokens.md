## Issuing tokens

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_token_issuer

The Breez SDK provides a specialized Token Issuer interface for managing custom token issuance on the Spark network using the using the [BTKN protocol](https://docs.spark.money/learn/tokens/hello-btkn). This functionality enables token creators to issue, manage, and control their own tokens with advanced features.

```dart
TokenIssuer tokenIssuer = sdk.getTokenIssuer();
```



## Token creation

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.create_issuer_token

Create a custom token with configurable parameters. Define the decimal precision, max supply and if the token can be frozen.

```dart
CreateIssuerTokenRequest request = CreateIssuerTokenRequest(
  name: "My Token",
  ticker: "MTK",
  decimals: 6,
  isFreezable: false,
  maxSupply: BigInt.from(1000000),
);
TokenMetadata tokenMetadata =
    await tokenIssuer.createIssuerToken(request: request);
print("Token identifier: ${tokenMetadata.identifier}");
```



### Creating multiple tokens

Token creation is limited to one token per issuer wallet. If you need to create and then manage more than one token using the same mnemonic, we recommend using different account numbers when initializing the SDK.

```dart
var accountNumber = 21;

String mnemonic = "<mnemonic words>";
final seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: null);
final config = defaultConfig(network: Network.mainnet)
    .copyWith(apiKey: "<breez api key>");
final builder = SdkBuilder(config: config, seed: seed);
builder.withDefaultStorage(storageDir: "./.data");

// Set the account number for the SDK
builder.withAccountNumber(accountNumber: accountNumber);

var sdk = await builder.build();
```



## Supply Management

### Minting a token

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.mint_issuer_token

Mint to increase the circulating supply of the token.

```dart
MintIssuerTokenRequest request = MintIssuerTokenRequest(
  amount: BigInt.from(1000),
);
Payment payment = await tokenIssuer.mintIssuerToken(request: request);
```



### Burning a token

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.burn_issuer_token

Permanently remove tokens from the circulating supply by burning them.

```dart
BurnIssuerTokenRequest request = BurnIssuerTokenRequest(
  amount: BigInt.from(1000),
);
Payment payment = await tokenIssuer.burnIssuerToken(request: request);
```



### Listing mint or burn payments

Mint or burn payments are included in the regular payment history that is obtained when [Listing payments](./list_payments.md).

You can filter by token transaction type to only include mint, burn or transfer payments. Transfer payments are regular token payments that are not mint or burn payments.

```dart
// Provide one or multiple of the following filters to
// the `paymentDetailsFilter` field when listing payments
PaymentDetailsFilter paymentDetailsTransferFilter =
    PaymentDetailsFilter.token(txType: TokenTransactionType.transfer);
PaymentDetailsFilter paymentDetailsMintFilter =
    PaymentDetailsFilter.token(txType: TokenTransactionType.mint);
PaymentDetailsFilter paymentDetailsBurnFilter =
    PaymentDetailsFilter.token(txType: TokenTransactionType.burn);
```



## Query balance & metadata

Retrieve the current issued token balance and fetch the token metadata.

```dart
TokenBalance tokenBalance = await tokenIssuer.getIssuerTokenBalance();
print("Token balance: ${tokenBalance.balance}");

TokenMetadata tokenMetadata = await tokenIssuer.getIssuerTokenMetadata();
print("Token ticker: ${tokenMetadata.ticker}");
```



## Freeze and unfreeze tokens

Freeze and unfreeze tokens at a specific Spark address if the token metadata allows it.

```dart
String sparkAddress = "<spark address>";
// Freeze the tokens held at the specified Spark address
FreezeIssuerTokenRequest freezeRequest =
    FreezeIssuerTokenRequest(address: sparkAddress);
FreezeIssuerTokenResponse freezeResponse =
    await tokenIssuer.freezeIssuerToken(request: freezeRequest);

// Unfreeze the tokens held at the specified Spark address
UnfreezeIssuerTokenRequest unfreezeRequest =
    UnfreezeIssuerTokenRequest(address: sparkAddress);
UnfreezeIssuerTokenResponse unfreezeResponse =
    await tokenIssuer.unfreezeIssuerToken(request: unfreezeRequest);
```
