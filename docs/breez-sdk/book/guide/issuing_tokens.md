## Issuing tokens

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_token_issuer

The Breez SDK provides a specialized Token Issuer interface for managing custom token issuance on the Spark network using the using the [BTKN protocol](https://docs.spark.money/learn/tokens/hello-btkn). This functionality enables token creators to issue, manage, and control their own tokens with advanced features.

### Rust

```rust
let token_issuer = sdk.get_token_issuer();
```

### Swift

```swift
let tokenIssuer = sdk.getTokenIssuer()
```

### Kotlin

```kotlin
val tokenIssuer = sdk.getTokenIssuer()
```

### C#

```csharp
var tokenIssuer = sdk.GetTokenIssuer();
```

### Javascript (Wasm)

```typescript
const tokenIssuer = sdk.getTokenIssuer()
```

### React Native

```typescript
const tokenIssuer = sdk.getTokenIssuer()
```

### Flutter

```dart
TokenIssuer tokenIssuer = sdk.getTokenIssuer();
```

### Python

```python
token_issuer = sdk.get_token_issuer()
```

### Go

```go
tokenIssuer := sdk.GetTokenIssuer()
```



## Token creation

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.create_issuer_token

Create a custom token with configurable parameters. Define the decimal precision, max supply and if the token can be frozen.

### Rust

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

### Swift

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

### Kotlin

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

### C#

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

### Javascript (Wasm)

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

### React Native

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

### Flutter

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

### Python

```python
try:
    request = CreateIssuerTokenRequest(
        name="My Token",
        ticker="MTK",
        decimals=6,
        is_freezable=False,
        max_supply=1_000_000,
    )
    token_metadata = await token_issuer.create_issuer_token(request)
    logging.debug(f"Token identifier: {token_metadata.identifier}")
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
request := breez_sdk_spark.CreateIssuerTokenRequest{
	Name:        "My Token",
	Ticker:      "MTK",
	Decimals:    6,
	IsFreezable: false,
	MaxSupply:   new(big.Int).SetInt64(1_000_000),
}
tokenMetadata, err := tokenIssuer.CreateIssuerToken(request)
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}
log.Printf("Token identifier: %v", tokenMetadata.Identifier)
```



### Creating multiple tokens

Token creation is limited to one token per issuer wallet. If you need to create and then manage more than one token using the same mnemonic, we recommend using different account numbers when initializing the SDK.

#### Rust

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

#### Swift

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

#### Kotlin

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

#### C#

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

#### Javascript (Wasm)

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

#### React Native

```typescript
const accountNumber = 21

const mnemonic = '<mnemonics words>'
const seed = new Seed.Mnemonic({ mnemonic, passphrase: undefined })
const config = defaultConfig(Network.Mainnet)
config.apiKey = '<breez api key>'
const builder = new SdkBuilder(config, seed)
await builder.withDefaultStorage(`${RNFS.DocumentDirectoryPath}/data`)

// Set the account number for the SDK
await builder.withAccountNumber(accountNumber)

const sdk = await builder.build()
```

#### Flutter

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

#### Python

```python
account_number = 21

mnemonic = "<mnemonic words>"
seed = Seed.MNEMONIC(mnemonic=mnemonic, passphrase=None)
config = default_config(network=Network.MAINNET)
config.api_key = "<breez api key>"
try:
    builder = SdkBuilder(config=config, seed=seed)
    await builder.with_default_storage(storage_dir="./.data")

    # Set the account number for the SDK
    await builder.with_account_number(account_number=account_number)

    sdk = await builder.build()
    return sdk
except Exception as error:
    logging.error(error)
    raise
```

#### Go

```go
accountNumber := uint32(21)

mnemonic := "<mnemonic words>"
var seed breez_sdk_spark.Seed = breez_sdk_spark.SeedMnemonic{
	Mnemonic:   mnemonic,
	Passphrase: nil,
}
apiKey := "<breez api key>"
config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
config.ApiKey = &apiKey
builder := breez_sdk_spark.NewSdkBuilder(config, seed)
builder.WithDefaultStorage("./.data")

// Set the account number for the SDK
builder.WithAccountNumber(accountNumber)

sdk, err := builder.Build()
```



## Supply Management

### Minting a token

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.mint_issuer_token

Mint to increase the circulating supply of the token.

#### Rust

```rust
let request = MintIssuerTokenRequest { amount: 1_000 };

let payment = token_issuer.mint_issuer_token(request).await?;
```

#### Swift

```swift
let request = MintIssuerTokenRequest(
    amount: BInt(1_000)
)
let payment = try await tokenIssuer.mintIssuerToken(request: request)
```

#### Kotlin

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

#### C#

```csharp
var amount = new BigInteger(1000);
var request = new MintIssuerTokenRequest(
    amount: amount
);
var payment = await tokenIssuer.MintIssuerToken(request);
```

#### Javascript (Wasm)

```typescript
const payment = await tokenIssuer.mintIssuerToken({
  amount: BigInt(1_000)
})
```

#### React Native

```typescript
const payment = await tokenIssuer.mintIssuerToken({
  amount: BigInt(1_000)
})
```

#### Flutter

```dart
MintIssuerTokenRequest request = MintIssuerTokenRequest(
  amount: BigInt.from(1000),
);
Payment payment = await tokenIssuer.mintIssuerToken(request: request);
```

#### Python

```python
try:
    request = MintIssuerTokenRequest(
        amount=1_000,
    )
    payment = await token_issuer.mint_issuer_token(request)
except Exception as error:
    logging.error(error)
    raise
```

#### Go

```go
request := breez_sdk_spark.MintIssuerTokenRequest{
	Amount: new(big.Int).SetInt64(1_000),
}
payment, err := tokenIssuer.MintIssuerToken(request)
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}
```



### Burning a token

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.burn_issuer_token

Permanently remove tokens from the circulating supply by burning them.

#### Rust

```rust
let request = BurnIssuerTokenRequest { amount: 1_000 };

let payment = token_issuer.burn_issuer_token(request).await?;
```

#### Swift

```swift
let request = BurnIssuerTokenRequest(
    amount: BInt(1_000)
)
let payment = try await tokenIssuer.burnIssuerToken(request: request)
```

#### Kotlin

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

#### C#

```csharp
var amount = new BigInteger(1000);
var request = new BurnIssuerTokenRequest(
    amount: amount
);
var payment = await tokenIssuer.BurnIssuerToken(request);
```

#### Javascript (Wasm)

```typescript
const payment = await tokenIssuer.burnIssuerToken({
  amount: BigInt(1_000)
})
```

#### React Native

```typescript
const payment = await tokenIssuer.burnIssuerToken({
  amount: BigInt(1_000)
})
```

#### Flutter

```dart
BurnIssuerTokenRequest request = BurnIssuerTokenRequest(
  amount: BigInt.from(1000),
);
Payment payment = await tokenIssuer.burnIssuerToken(request: request);
```

#### Python

```python
try:
    request = BurnIssuerTokenRequest(
        amount=1_000,
    )
    payment = await token_issuer.burn_issuer_token(request)
except Exception as error:
    logging.error(error)
    raise
```

#### Go

```go
request := breez_sdk_spark.BurnIssuerTokenRequest{
	Amount: new(big.Int).SetInt64(1_000),
}
payment, err := tokenIssuer.BurnIssuerToken(request)
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}
```



### Listing mint or burn payments

Mint or burn payments are included in the regular payment history that is obtained when [Listing payments](./list_payments.md).

You can filter by token transaction type to only include mint, burn or transfer payments. Transfer payments are regular token payments that are not mint or burn payments.

#### Rust

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

#### Swift

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

#### Kotlin

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

#### C#

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

#### Javascript (Wasm)

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

#### React Native

```typescript
// Provide one or multiple of the following filters to
// the `paymentDetailsFilter` field when listing payments
const paymentDetailsTransferFilter = new PaymentDetailsFilter.Token({
  txType: TokenTransactionType.Transfer,
  txHash: undefined,
  conversionRefundNeeded: undefined
})
const paymentDetailsMintFilter = new PaymentDetailsFilter.Token({
  txType: TokenTransactionType.Mint,
  txHash: undefined,
  conversionRefundNeeded: undefined
})
const paymentDetailsBurnFilter = new PaymentDetailsFilter.Token({
  txType: TokenTransactionType.Burn,
  txHash: undefined,
  conversionRefundNeeded: undefined
})
```

#### Flutter

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

#### Python

```python
# Provide one or multiple of the following filters to
# the `payment_details_filter` field when listing payments
payment_details_transfer_filter = PaymentDetailsFilter.TOKEN(
    tx_type=TokenTransactionType.TRANSFER,
    tx_hash=None,
    conversion_refund_needed=None
)
payment_details_mint_filter = PaymentDetailsFilter.TOKEN(
    tx_type=TokenTransactionType.MINT,
    tx_hash=None,
    conversion_refund_needed=None
)
payment_details_burn_filter = PaymentDetailsFilter.TOKEN(
    tx_type=TokenTransactionType.BURN,
    tx_hash=None,
    conversion_refund_needed=None
)
```

#### Go

```go
// Provide one or multiple of the following filters to
// the `PaymentDetailsFilter` field when listing payments
transferType := breez_sdk_spark.TokenTransactionTypeTransfer
mintType := breez_sdk_spark.TokenTransactionTypeMint
burnType := breez_sdk_spark.TokenTransactionTypeBurn
paymentDetailsTransferFilter := breez_sdk_spark.PaymentDetailsFilterToken{TxType: &transferType}
paymentDetailsMintFilter := breez_sdk_spark.PaymentDetailsFilterToken{TxType: &mintType}
paymentDetailsBurnFilter := breez_sdk_spark.PaymentDetailsFilterToken{TxType: &burnType}
```



## Query balance & metadata

Retrieve the current issued token balance and fetch the token metadata.

### Rust

```rust
let token_balance = token_issuer.get_issuer_token_balance().await?;
info!("Token balance: {}", token_balance.balance);

let token_metadata = token_issuer.get_issuer_token_metadata().await?;
info!("Token ticker: {}", token_metadata.ticker);
```

### Swift

```swift
let tokenBalance = try await tokenIssuer.getIssuerTokenBalance()
print("Token balance: {}", tokenBalance.balance)

let tokenMetadata = try await tokenIssuer.getIssuerTokenMetadata()
print("Token ticker: {}", tokenMetadata.ticker)
```

### Kotlin

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

### C#

```csharp
var tokenBalance = await tokenIssuer.GetIssuerTokenBalance();
Console.WriteLine($"Token balance: {tokenBalance.balance}");

var tokenMetadata = await tokenIssuer.GetIssuerTokenMetadata();
Console.WriteLine($"Token ticker: {tokenMetadata.ticker}");
```

### Javascript (Wasm)

```typescript
const tokenBalance = await tokenIssuer.getIssuerTokenBalance()
console.debug(`Token balance: ${tokenBalance.balance}`)

const tokenMetadata = await tokenIssuer.getIssuerTokenMetadata()
console.debug(`Token ticker: ${tokenMetadata.ticker}`)
```

### React Native

```typescript
const tokenBalance = await tokenIssuer.getIssuerTokenBalance()
console.debug(`Token balance: ${tokenBalance.balance}`)

const tokenMetadata = await tokenIssuer.getIssuerTokenMetadata()
console.debug(`Token ticker: ${tokenMetadata.ticker}`)
```

### Flutter

```dart
TokenBalance tokenBalance = await tokenIssuer.getIssuerTokenBalance();
print("Token balance: ${tokenBalance.balance}");

TokenMetadata tokenMetadata = await tokenIssuer.getIssuerTokenMetadata();
print("Token ticker: ${tokenMetadata.ticker}");
```

### Python

```python
try:
    token_balance = await token_issuer.get_issuer_token_balance()
    logging.debug(f"Token balance: {token_balance.balance}")

    token_metadata = await token_issuer.get_issuer_token_metadata()
    logging.debug(f"Token ticker: {token_metadata.ticker}")
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
tokenBalance, err := tokenIssuer.GetIssuerTokenBalance()
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}
log.Printf("Token balance: %v", tokenBalance.Balance)

tokenMetadata, err := tokenIssuer.GetIssuerTokenMetadata()
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}
log.Printf("Token ticker: %v", tokenMetadata.Ticker)
```



## Freeze and unfreeze tokens

Freeze and unfreeze tokens at a specific Spark address if the token metadata allows it.

### Rust

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

### Swift

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

### Kotlin

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

### C#

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

### Javascript (Wasm)

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

### React Native

```typescript
const sparkAddress = '<spark address>'
// Freeze the tokens held at the specified Spark address
const freezeResponse = await tokenIssuer.freezeIssuerToken({
  address: sparkAddress
})

// To unfreeze the tokens, use the following:
const unfreezeResponse = await tokenIssuer.unfreezeIssuerToken({
  address: sparkAddress
})
```

### Flutter

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

### Python

```python
try:
    spark_address = "<spark address>"
    # Freeze the tokens held at the specified Spark address
    freeze_request = FreezeIssuerTokenRequest(
        address=spark_address,
    )
    freeze_response = await token_issuer.freeze_issuer_token(freeze_request)
    # Unfreeze the tokens held at the specified Spark address
    unfreeze_request = UnfreezeIssuerTokenRequest(
        address=spark_address,
    )
    unfreeze_response = await token_issuer.unfreeze_issuer_token(unfreeze_request)
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
sparkAddress := "<spark address>"
// Freeze the tokens held at the specified Spark address
freezeRequest := breez_sdk_spark.FreezeIssuerTokenRequest{
	Address: sparkAddress,
}
freezeResponse, err := tokenIssuer.FreezeIssuerToken(freezeRequest)
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}

// Unfreeze the tokens held at the specified Spark address
unfreezeRequest := breez_sdk_spark.UnfreezeIssuerTokenRequest{
	Address: sparkAddress,
}
unfreezeResponse, err := tokenIssuer.UnfreezeIssuerToken(unfreezeRequest)
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}
```

