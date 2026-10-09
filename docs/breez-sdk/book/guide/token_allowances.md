# Token allowances

A token allowance lets another wallet pull tokens from your wallet, within limits you set. The wallet that grants the allowance is the owner. The wallet that pulls is the spender.

An allowance names the spender by its Spark address, and sets:

- the token
- a limit on each payment and a limit on all payments together
- an expiry
- an optional list of recipients the spender may pay

The owner can revoke an allowance at any time. An allowance doesn't reserve funds: a pull fails if the owner's balance is too low. In the owner's history, a pull shows up as an ordinary outgoing payment.

Granting and revoking need the device clock to be within about a minute of the network's time.

## Granting an allowance

### Rust

```rust
let response = sdk
    .create_token_allowance(CreateTokenAllowanceRequest {
        spender_address: "<spender spark address>".to_string(),
        token_identifier: "<token identifier>".to_string(),
        max_per_payment: TokenAllowanceLimit::Amount { amount: 5_000_000 },
        max_total: TokenAllowanceLimit::Amount {
            amount: 100_000_000,
        },
        expiry_time: 1_798_761_600,
        allowed_recipients: vec![],
    })
    .await?;
info!("Allowance id: {}", response.allowance.id);
```

### Swift

```swift
let response = try await sdk.createTokenAllowance(
    request: CreateTokenAllowanceRequest(
        spenderAddress: "<spender spark address>",
        tokenIdentifier: "<token identifier>",
        maxPerPayment: .amount(amount: BInt(5_000_000)),
        maxTotal: .amount(amount: BInt(100_000_000)),
        expiryTime: 1_798_761_600,
        allowedRecipients: []
    ))
print("Allowance id: \(response.allowance.id)")
```

### Kotlin

```kotlin
val response = sdk.createTokenAllowance(
    CreateTokenAllowanceRequest(
        spenderAddress = "<spender spark address>",
        tokenIdentifier = "<token identifier>",
        maxPerPayment = TokenAllowanceLimit.Amount(BigInteger.fromLong(5_000_000L)),
        maxTotal = TokenAllowanceLimit.Amount(BigInteger.fromLong(100_000_000L)),
        expiryTime = 1_798_761_600uL,
        allowedRecipients = emptyList(),
    )
)
// Log.v("Breez", "Allowance id: ${response.allowance.id}")
```

### C#

```csharp
var response = await sdk.CreateTokenAllowance(
    request: new CreateTokenAllowanceRequest(
        spenderAddress: "<spender spark address>",
        tokenIdentifier: "<token identifier>",
        maxPerPayment: new TokenAllowanceLimit.Amount(new BigInteger(5_000_000)),
        maxTotal: new TokenAllowanceLimit.Amount(new BigInteger(100_000_000)),
        expiryTime: 1_798_761_600UL,
        allowedRecipients: new string[] { }
    )
);
Console.WriteLine($"Allowance id: {response.allowance.id}");
```

### Javascript (Wasm)

```typescript
const response = await sdk.createTokenAllowance({
  spenderAddress: '<spender spark address>',
  tokenIdentifier: '<token identifier>',
  maxPerPayment: { type: 'amount', amount: '5000000' },
  maxTotal: { type: 'amount', amount: '100000000' },
  expiryTime: 1_798_761_600,
  allowedRecipients: []
})
console.log(`Allowance id: ${response.allowance.id}`)
```

### React Native

```typescript
const response = await sdk.createTokenAllowance({
  spenderAddress: '<spender spark address>',
  tokenIdentifier: '<token identifier>',
  maxPerPayment: new TokenAllowanceLimit.Amount({ amount: BigInt(5_000_000) }),
  maxTotal: new TokenAllowanceLimit.Amount({ amount: BigInt(100_000_000) }),
  expiryTime: BigInt(1_798_761_600),
  allowedRecipients: []
})
console.log(`Allowance id: ${response.allowance.id}`)
```

### Flutter

```dart
final response = await sdk.createTokenAllowance(
    request: CreateTokenAllowanceRequest(
        spenderAddress: "<spender spark address>",
        tokenIdentifier: "<token identifier>",
        maxPerPayment: TokenAllowanceLimit.amount(amount: BigInt.from(5000000)),
        maxTotal: TokenAllowanceLimit.amount(amount: BigInt.from(100000000)),
        expiryTime: BigInt.from(1798761600),
        allowedRecipients: []));
print("Allowance id: ${response.allowance.id}");
```

### Python

```python
try:
    response = await sdk.create_token_allowance(
        request=CreateTokenAllowanceRequest(
            spender_address="<spender spark address>",
            token_identifier="<token identifier>",
            max_per_payment=TokenAllowanceLimit.AMOUNT(amount=5_000_000),
            max_total=TokenAllowanceLimit.AMOUNT(amount=100_000_000),
            expiry_time=1_798_761_600,
            allowed_recipients=[],
        )
    )
    logging.debug(f"Allowance id: {response.allowance.id}")
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
maxPerPayment := breez_sdk_spark.TokenAllowanceLimitAmount{
	Amount: new(big.Int).SetInt64(5_000_000),
}
maxTotal := breez_sdk_spark.TokenAllowanceLimitAmount{
	Amount: new(big.Int).SetInt64(100_000_000),
}

response, err := sdk.CreateTokenAllowance(breez_sdk_spark.CreateTokenAllowanceRequest{
	SpenderAddress:    "<spender spark address>",
	TokenIdentifier:   "<token identifier>",
	MaxPerPayment:     maxPerPayment,
	MaxTotal:          maxTotal,
	ExpiryTime:        1_798_761_600,
	AllowedRecipients: []string{},
})
if err != nil {
	return err
}
log.Printf("Allowance id: %v", response.Allowance.Id)
```



Both limits must be given: to leave one open, set it to `TokenAllowanceLimit::Unlimited`.

Only one allowance can be active for the same spender and token. Granting another returns `TokenAllowanceErrorReason::AlreadyActive`: list your allowances to find the active one. To change the limits, revoke the allowance and grant a new one.

## Listing allowances

Use `TokenAllowanceRole::Owner` for allowances your wallet granted and `TokenAllowanceRole::Spender` for allowances granted to it.

### Rust

```rust
let response = sdk
    .list_token_allowances(ListTokenAllowancesRequest {
        role: TokenAllowanceRole::Owner,
        counterparty_address: None,
        token_identifier: None,
        include_inactive: Some(false),
        offset: None,
        limit: None,
    })
    .await?;
for allowance in response.allowances {
    info!("{}: spent {}", allowance.id, allowance.spent_amount);
}
```

### Swift

```swift
let response = try await sdk.listTokenAllowances(
    request: ListTokenAllowancesRequest(
        role: .owner,
        counterpartyAddress: nil,
        tokenIdentifier: nil,
        includeInactive: false,
        offset: nil,
        limit: nil
    ))
for allowance in response.allowances {
    print("\(allowance.id): spent \(allowance.spentAmount)")
}
```

### Kotlin

```kotlin
val response = sdk.listTokenAllowances(
    ListTokenAllowancesRequest(
        role = TokenAllowanceRole.OWNER,
        counterpartyAddress = null,
        tokenIdentifier = null,
        includeInactive = false,
        offset = null,
        limit = null,
    )
)
for (allowance in response.allowances) {
    // Log.v("Breez", "${allowance.id}: spent ${allowance.spentAmount}")
}
```

### C#

```csharp
var response = await sdk.ListTokenAllowances(
    request: new ListTokenAllowancesRequest(
        role: TokenAllowanceRole.Owner,
        counterpartyAddress: null,
        tokenIdentifier: null,
        includeInactive: false,
        offset: null,
        limit: null
    )
);
foreach (var allowance in response.allowances)
{
    Console.WriteLine($"{allowance.id}: spent {allowance.spentAmount}");
}
```

### Javascript (Wasm)

```typescript
const response = await sdk.listTokenAllowances({ role: 'owner', includeInactive: false })
for (const allowance of response.allowances) {
  console.log(`${allowance.id}: spent ${allowance.spentAmount}`)
}
```

### React Native

```typescript
const response = await sdk.listTokenAllowances({
  role: TokenAllowanceRole.Owner,
  counterpartyAddress: undefined,
  tokenIdentifier: undefined,
  includeInactive: false,
  offset: undefined,
  limit: undefined
})
for (const allowance of response.allowances) {
  console.log(`${allowance.id}: spent ${allowance.spentAmount}`)
}
```

### Flutter

```dart
final response = await sdk.listTokenAllowances(
    request: ListTokenAllowancesRequest(
        role: TokenAllowanceRole.owner,
        counterpartyAddress: null,
        tokenIdentifier: null,
        includeInactive: false,
        offset: null,
        limit: null));
for (final allowance in response.allowances) {
  print("${allowance.id}: spent ${allowance.spentAmount}");
}
```

### Python

```python
try:
    response = await sdk.list_token_allowances(
        request=ListTokenAllowancesRequest(
            role=TokenAllowanceRole.OWNER,
            counterparty_address=None,
            token_identifier=None,
            include_inactive=False,
            offset=None,
            limit=None,
        )
    )
    for allowance in response.allowances:
        logging.debug(f"{allowance.id}: spent {allowance.spent_amount}")
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
includeInactive := false

response, err := sdk.ListTokenAllowances(breez_sdk_spark.ListTokenAllowancesRequest{
	Role:                breez_sdk_spark.TokenAllowanceRoleOwner,
	CounterpartyAddress: nil,
	TokenIdentifier:     nil,
	IncludeInactive:     &includeInactive,
	Offset:              nil,
	Limit:               nil,
})
if err != nil {
	return err
}
for _, allowance := range response.Allowances {
	log.Printf("%v: spent %v", allowance.Id, allowance.SpentAmount)
}
```



The spent amount counts a pull as soon as it is sent, including pulls that later fail, so it can be higher than what was actually pulled.

## Revoking an allowance

### Rust

```rust
sdk.revoke_token_allowance(RevokeTokenAllowanceRequest {
    allowance_id: "<allowance id>".to_string(),
})
.await?;
```

### Swift

```swift
try await sdk.revokeTokenAllowance(
    request: RevokeTokenAllowanceRequest(allowanceId: "<allowance id>"))
```

### Kotlin

```kotlin
sdk.revokeTokenAllowance(RevokeTokenAllowanceRequest(allowanceId = "<allowance id>"))
```

### C#

```csharp
await sdk.RevokeTokenAllowance(
    request: new RevokeTokenAllowanceRequest(allowanceId: "<allowance id>")
);
```

### Javascript (Wasm)

```typescript
await sdk.revokeTokenAllowance({ allowanceId: '<allowance id>' })
```

### React Native

```typescript
await sdk.revokeTokenAllowance({ allowanceId: '<allowance id>' })
```

### Flutter

```dart
await sdk.revokeTokenAllowance(
    request: RevokeTokenAllowanceRequest(allowanceId: "<allowance id>"));
```

### Python

```python
try:
    await sdk.revoke_token_allowance(
        request=RevokeTokenAllowanceRequest(allowance_id="<allowance id>")
    )
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
err := sdk.RevokeTokenAllowance(breez_sdk_spark.RevokeTokenAllowanceRequest{
	AllowanceId: "<allowance id>",
})
if err != nil {
	return err
}
```



A revocation takes effect within seconds. A pull that hasn't completed can still fail after a revocation, so treat a pull as paid only once it completes.

## Pulling a payment

A spender pulls in two steps. `prepare_pull_payment` finds the allowance, checks its limits and builds the transfer from the payer's tokens. `pull_payment` signs it and sends it.

### Rust

```rust
let prepare_response = sdk
    .prepare_pull_payment(PreparePullPaymentRequest {
        payer_address: "<payer spark address>".to_string(),
        token_identifier: "<token identifier>".to_string(),
        receivers: vec![PullReceiver {
            amount: 5_000_000,
            receiver_address: None,
        }],
    })
    .await?;
info!("Pulling {}", prepare_response.amount);

let response = sdk
    .pull_payment(PullPaymentRequest { prepare_response })
    .await?;
info!("Pull transaction: {}", response.tx_hash);
```

### Swift

```swift
let prepareResponse = try await sdk.preparePullPayment(
    request: PreparePullPaymentRequest(
        payerAddress: "<payer spark address>",
        tokenIdentifier: "<token identifier>",
        receivers: [
            PullReceiver(amount: BInt(5_000_000), receiverAddress: nil)
        ]
    ))
print("Pulling \(prepareResponse.amount)")

let response = try await sdk.pullPayment(
    request: PullPaymentRequest(prepareResponse: prepareResponse))
print("Pull transaction: \(response.txHash)")
```

### Kotlin

```kotlin
val prepareResponse = sdk.preparePullPayment(
    PreparePullPaymentRequest(
        payerAddress = "<payer spark address>",
        tokenIdentifier = "<token identifier>",
        receivers = listOf(
            PullReceiver(
                amount = BigInteger.fromLong(5_000_000L),
                receiverAddress = null,
            )
        ),
    )
)
// Log.v("Breez", "Pulling ${prepareResponse.amount}")

val response = sdk.pullPayment(PullPaymentRequest(prepareResponse))
// Log.v("Breez", "Pull transaction: ${response.txHash}")
```

### C#

```csharp
var prepareResponse = await sdk.PreparePullPayment(
    request: new PreparePullPaymentRequest(
        payerAddress: "<payer spark address>",
        tokenIdentifier: "<token identifier>",
        receivers: new PullReceiver[] {
            new PullReceiver(
                amount: new BigInteger(5_000_000),
                receiverAddress: null
            )
        }
    )
);
Console.WriteLine($"Pulling {prepareResponse.amount}");

var response = await sdk.PullPayment(
    request: new PullPaymentRequest(prepareResponse: prepareResponse)
);
Console.WriteLine($"Pull transaction: {response.txHash}");
```

### Javascript (Wasm)

```typescript
const prepareResponse = await sdk.preparePullPayment({
  payerAddress: '<payer spark address>',
  tokenIdentifier: '<token identifier>',
  receivers: [{ amount: '5000000' }]
})
console.log(`Pulling ${prepareResponse.amount}`)

const response = await sdk.pullPayment({ prepareResponse })
console.log(`Pull transaction: ${response.txHash}`)
```

### React Native

```typescript
const prepareResponse = await sdk.preparePullPayment({
  payerAddress: '<payer spark address>',
  tokenIdentifier: '<token identifier>',
  receivers: [{ amount: BigInt(5_000_000), receiverAddress: undefined }]
})
console.log(`Pulling ${prepareResponse.amount}`)

const response = await sdk.pullPayment({ prepareResponse })
console.log(`Pull transaction: ${response.txHash}`)
```

### Flutter

```dart
final prepareResponse = await sdk.preparePullPayment(
    request: PreparePullPaymentRequest(
        payerAddress: "<payer spark address>",
        tokenIdentifier: "<token identifier>",
        receivers: [
      PullReceiver(amount: BigInt.from(5000000), receiverAddress: null)
    ]));
print("Pulling ${prepareResponse.amount}");

final response = await sdk.pullPayment(
    request: PullPaymentRequest(prepareResponse: prepareResponse));
print("Pull transaction: ${response.txHash}");
```

### Python

```python
try:
    prepare_response = await sdk.prepare_pull_payment(
        request=PreparePullPaymentRequest(
            payer_address="<payer spark address>",
            token_identifier="<token identifier>",
            receivers=[PullReceiver(amount=5_000_000, receiver_address=None)],
        )
    )
    logging.debug(f"Pulling {prepare_response.amount}")

    response = await sdk.pull_payment(
        request=PullPaymentRequest(prepare_response=prepare_response)
    )
    logging.debug(f"Pull transaction: {response.tx_hash}")
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
prepareResponse, err := sdk.PreparePullPayment(breez_sdk_spark.PreparePullPaymentRequest{
	PayerAddress:    "<payer spark address>",
	TokenIdentifier: "<token identifier>",
	Receivers: []breez_sdk_spark.PullReceiver{
		{
			Amount:          new(big.Int).SetInt64(5_000_000),
			ReceiverAddress: nil,
		},
	},
})
if err != nil {
	return err
}
log.Printf("Pulling %v", prepareResponse.Amount)

response, err := sdk.PullPayment(breez_sdk_spark.PullPaymentRequest{
	PrepareResponse: prepareResponse,
})
if err != nil {
	return err
}
log.Printf("Pull transaction: %v", response.TxHash)
```



Leave a receiver's address unset to pull into your own wallet. One pull can pay several receivers. When the allowance has a recipient list, every receiver must be on it, your own wallet included. The per-payment limit applies to the total.

### Retrying

A pull counts as paid once `pull_payment` returns it with the `status` `PaymentStatus::Completed`. A `PaymentStatus::Pending` pull is accepted but can still fail: call `pull_payment` again with the same prepare response to refresh it.

If `pull_payment` fails with `SdkError::SparkError` or `SdkError::NetworkError`, call it again with the same prepare response; the operators recognize a pull they've already seen, so this never charges the payer twice. Keep calling until the pull completes, fails with `SdkError::TokenAllowance`, or 10 minutes have passed since its `expiry_time`, because a pull the operators accepted can still complete for a few minutes after that time. Prepare a new pull only after that.

`TokenAllowanceErrorReason::PreparedPullStale` means the operators never accepted the pull and never will: the tokens it was prepared with were spent by another payment, or it wasn't sent before its window closed. Prepare a new pull right away. Don't call `pull_payment` twice at the same time with the same prepare response, because the second call can report a pull the first one is still sending as stale.

## Errors

Token allowance errors return `SdkError::TokenAllowance` with a `reason` that says why the request was refused, for example `TokenAllowanceErrorReason::OverPerPaymentLimit`, `TokenAllowanceErrorReason::OverTotalLimit`, `TokenAllowanceErrorReason::RecipientNotAllowed`, `TokenAllowanceErrorReason::Revoked` or `TokenAllowanceErrorReason::PayerInsufficientFunds`. `TokenAllowanceErrorReason::PreparedPullStale` means the pull can't go through anymore (see [Retrying](#retrying)). `TokenAllowanceErrorReason::NotEnabled` means token allowances aren't available on this network.

In JavaScript, errors arrive as text: the reason's name is part of the error message.

## Signing pulls on the client

To have the spender's key approve each pull on a separate device or signing service, see [Client signing](client_signing.md#pulls).

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
