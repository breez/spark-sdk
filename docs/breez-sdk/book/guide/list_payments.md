# Listing payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.list_payments

To view your payment history, you can list all the payments that have been sent and received.

## Rust

```rust
let response = sdk.list_payments(ListPaymentsRequest::default()).await?;
let payments = response.payments;
```

## Swift

```swift
let response = try await sdk.listPayments(
    request: ListPaymentsRequest())
let payments = response.payments
```

## Kotlin

```kotlin
try {
    val response = sdk.listPayments(ListPaymentsRequest())
    val payments = response.payments
} catch (e: Exception) {
    // handle error
}
```

## C#

```csharp
var response = await sdk.ListPayments(request: new ListPaymentsRequest());
var payments = response.payments;
```

## Javascript (Wasm)

```typescript
const response = await sdk.listPayments({})
const payments = response.payments
```

## React Native

```typescript
const response = await sdk.listPayments({
  typeFilter: undefined,
  statusFilter: undefined,
  assetFilter: undefined,
  paymentDetailsFilter: undefined,
  fromTimestamp: undefined,
  toTimestamp: undefined,
  offset: undefined,
  limit: undefined,
  sortAscending: undefined
})
const payments = response.payments
```

## Flutter

```dart
ListPaymentsRequest request = ListPaymentsRequest();
ListPaymentsResponse response = await sdk.listPayments(request: request);
List<Payment> payments = response.payments;
```

## Python

```python
response = await sdk.list_payments(request=ListPaymentsRequest())
payments = response.payments
```

## Go

```go
response, err := sdk.ListPayments(breez_sdk_spark.ListPaymentsRequest{})

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

payments := response.Payments
```



## Filtering Payments

When listing payments you can also filter and page the results.

### Rust

```rust
// Filter by asset (Bitcoin or Token)
let asset_filter = AssetFilter::Token {
    token_identifier: Some("token_identifier_here".to_string()),
};
// To filter by Bitcoin instead:
// let asset_filter = AssetFilter::Bitcoin;

let response = sdk
    .list_payments(ListPaymentsRequest {
        // Filter by payment type
        type_filter: Some(vec![PaymentType::Send, PaymentType::Receive]),
        // Filter by status
        status_filter: Some(vec![PaymentStatus::Completed]),
        asset_filter: Some(asset_filter),
        // Time range filters
        from_timestamp: Some(1704067200), // Unix timestamp
        to_timestamp: Some(1735689600),   // Unix timestamp
        // Pagination
        offset: Some(0),
        limit: Some(50),
        // Sort order (true = oldest first, false = newest first)
        sort_ascending: Some(false),
        payment_details_filter: None,
    })
    .await?;
let payments = response.payments;
```

### Swift

```swift
// Filter by asset (Bitcoin or Token)
let assetFilter = AssetFilter.token(tokenIdentifier: "token_identifier_here")
// To filter by Bitcoin instead:
// let assetFilter = AssetFilter.bitcoin

let response = try await sdk.listPayments(
    request: ListPaymentsRequest(
        // Filter by payment type
        typeFilter: [PaymentType.send, PaymentType.receive],
        // Filter by status
        statusFilter: [PaymentStatus.completed],
        assetFilter: assetFilter,
        // Time range filters
        fromTimestamp: 1_704_067_200,  // Unix timestamp
        toTimestamp: 1_735_689_600,  // Unix timestamp
        // Pagination
        offset: 0,
        limit: 50,
        // Sort order (true = oldest first, false = newest first)
        sortAscending: false
    ))
let payments = response.payments
```

### Kotlin

```kotlin
try {
    // Filter by asset (Bitcoin or Token)
    val assetFilter = AssetFilter.Token(tokenIdentifier = "token_identifier_here")
    // To filter by Bitcoin instead:
    // val assetFilter = AssetFilter.Bitcoin

    val response = sdk.listPayments(
        ListPaymentsRequest(
            // Filter by payment type
            typeFilter = listOf(PaymentType.SEND, PaymentType.RECEIVE),
            // Filter by status
            statusFilter = listOf(PaymentStatus.COMPLETED),
            assetFilter = assetFilter,
            // Time range filters
            fromTimestamp = 1704067200u, // Unix timestamp
            toTimestamp = 1735689600u,   // Unix timestamp
            // Pagination
            offset = 0u,
            limit = 50u,
            // Sort order (true = oldest first, false = newest first)
            sortAscending = false
        ))
    val payments = response.payments
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
// Filter by asset (Bitcoin or Token)
var assetFilter = new AssetFilter.Token(tokenIdentifier: "token_identifier_here");
// To filter by Bitcoin instead:
// var assetFilter = new AssetFilter.Bitcoin();

var request = new ListPaymentsRequest(
    // Filter by payment type
    typeFilter: new PaymentType[] { PaymentType.Send, PaymentType.Receive },
    // Filter by status
    statusFilter: new PaymentStatus[] { PaymentStatus.Completed },
    assetFilter: assetFilter,
    // Time range filters
    fromTimestamp: 1704067200, // Unix timestamp
    toTimestamp: 1735689600,   // Unix timestamp
                               // Pagination
    offset: 0,
    limit: 50,
    // Sort order (true = oldest first, false = newest first)
    sortAscending: false
);

var response = await sdk.ListPayments(request: request);
var payments = response.payments;
```

### Javascript (Wasm)

```typescript
// Filter by asset (Bitcoin or Token)
const assetFilter: AssetFilter = { type: 'token', tokenIdentifier: 'token_identifier_here' }
// To filter by Bitcoin instead:
// const assetFilter: AssetFilter = { type: 'bitcoin' }

const response = await sdk.listPayments({
  // Filter by payment type
  typeFilter: ['send', 'receive'],
  // Filter by status
  statusFilter: ['completed'],
  assetFilter,
  // Time range filters
  fromTimestamp: 1704067200, // Unix timestamp
  toTimestamp: 1735689600, // Unix timestamp
  // Pagination
  offset: 0,
  limit: 50,
  // Sort order (true = oldest first, false = newest first)
  sortAscending: false
})
const payments = response.payments
```

### React Native

```typescript
// Filter by asset (Bitcoin or Token)
const assetFilter = new AssetFilter.Token({ tokenIdentifier: 'token_identifier_here' })
// To filter by Bitcoin instead:
// const assetFilter = new AssetFilter.Bitcoin()

const response = await sdk.listPayments({
  // Filter by payment type
  typeFilter: [PaymentType.Send, PaymentType.Receive],
  // Filter by status
  statusFilter: [PaymentStatus.Completed],
  assetFilter,
  paymentDetailsFilter: undefined,
  // Time range filters
  fromTimestamp: 1704067200n, // Unix timestamp
  toTimestamp: 1735689600n, // Unix timestamp
  // Pagination
  offset: 0,
  limit: 50,
  // Sort order (true = oldest first, false = newest first)
  sortAscending: false
})
const payments = response.payments
```

### Flutter

```dart
// Filter by asset (Bitcoin or Token)
AssetFilter assetFilter = AssetFilter.token(tokenIdentifier: "token_identifier_here");
// To filter by Bitcoin instead:
// AssetFilter assetFilter = AssetFilter.bitcoin();

ListPaymentsRequest request = ListPaymentsRequest(
  // Filter by payment type
  typeFilter: [PaymentType.send, PaymentType.receive],
  // Filter by status
  statusFilter: [PaymentStatus.completed],
  assetFilter: assetFilter,
  // Time range filters
  fromTimestamp: BigInt.from(1704067200), // Unix timestamp
  toTimestamp: BigInt.from(1735689600),   // Unix timestamp
  // Pagination
  offset: 0,
  limit: 50,
  // Sort order (true = oldest first, false = newest first)
  sortAscending: false,
);
ListPaymentsResponse response = await sdk.listPayments(request: request);
List<Payment> payments = response.payments;
```

### Python

```python
# Filter by asset (Bitcoin or Token)
asset_filter = AssetFilter.TOKEN(token_identifier="token_identifier_here")
# To filter by Bitcoin instead:
# asset_filter = AssetFilter.BITCOIN

request = ListPaymentsRequest(
    # Filter by payment type
    type_filter=[PaymentType.SEND, PaymentType.RECEIVE],
    # Filter by status
    status_filter=[PaymentStatus.COMPLETED],
    asset_filter=asset_filter,
    # Time range filters
    from_timestamp=1704067200,  # Unix timestamp
    to_timestamp=1735689600,    # Unix timestamp
    # Pagination
    offset=0,
    limit=50,
    # Sort order (true = oldest first, false = newest first)
    sort_ascending=False
)
response = await sdk.list_payments(request=request)
payments = response.payments
```

### Go

```go
// Filter by asset (Bitcoin or Token)
tokenIdentifier := "token_identifier_here"
var assetFilter breez_sdk_spark.AssetFilter = breez_sdk_spark.AssetFilterToken{
	TokenIdentifier: &tokenIdentifier,
}
// To filter by Bitcoin instead:
// var assetFilter breez_sdk_spark.AssetFilter = breez_sdk_spark.AssetFilterBitcoin

// Filter options
typeFilter := []breez_sdk_spark.PaymentType{
	breez_sdk_spark.PaymentTypeSend,
	breez_sdk_spark.PaymentTypeReceive,
}
statusFilter := []breez_sdk_spark.PaymentStatus{
	breez_sdk_spark.PaymentStatusCompleted,
}
fromTimestamp := uint64(1704067200) // Unix timestamp
toTimestamp := uint64(1735689600)   // Unix timestamp
offset := uint32(0)
limit := uint32(50)
sortAscending := false

request := breez_sdk_spark.ListPaymentsRequest{
	TypeFilter:    &typeFilter,    // Filter by payment type
	StatusFilter:  &statusFilter,  // Filter by status
	AssetFilter:   &assetFilter,   // Filter by asset (Bitcoin or Token)
	FromTimestamp: &fromTimestamp, // Time range filters
	ToTimestamp:   &toTimestamp,   // Time range filters
	Offset:        &offset,        // Pagination
	Limit:         &limit,         // Pagination
	SortAscending: &sortAscending, // Sort order (true = oldest first, false = newest first)
}
response, err := sdk.ListPayments(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

payments := response.Payments
```



## Get Payment

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_payment

You can also retrieve a single payment using the payment id:

### Rust

```rust
let payment_id = "<payment id>".to_string();
let response = sdk.get_payment(GetPaymentRequest { payment_id }).await?;
let payment = response.payment;
```

### Swift

```swift
let paymentId = "<payment id>"
let response = try await sdk.getPayment(
    request: GetPaymentRequest(paymentId: paymentId)
)
let payment = response.payment
```

### Kotlin

```kotlin
try {
    val paymentId = "<payment id>";
    val response = sdk.getPayment(GetPaymentRequest(paymentId))
    val payment = response.payment
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
var paymentId = "<payment id>";
var response = await sdk.GetPayment(
    request: new GetPaymentRequest(paymentId: paymentId)
);
var payment = response.payment;
```

### Javascript (Wasm)

```typescript
const paymentId = '<payment id>'
const response = await sdk.getPayment({
  paymentId
})
const payment = response.payment
```

### React Native

```typescript
const paymentId = '<payment id>'
const response = await sdk.getPayment({
  paymentId
})
const payment = response.payment
```

### Flutter

```dart
String paymentId = "<payment id>";
GetPaymentRequest request = GetPaymentRequest(paymentId: paymentId);
GetPaymentResponse response = await sdk.getPayment(request: request);
Payment payment = response.payment;
```

### Python

```python
payment_id = "<payment id>"
response = await sdk.get_payment(
    request=GetPaymentRequest(payment_id=payment_id)
)
payment = response.payment
```

### Go

```go
paymentId := "<payment id>"
request := breez_sdk_spark.GetPaymentRequest{
	PaymentId: paymentId,
}
response, err := sdk.GetPayment(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return nil, err
}

payment := response.Payment
```

