# Refunding deposits

A deposit that cannot or should not be claimed can be refunded to an external Bitcoin address. The refund sends the deposit amount, minus the transaction fee, to that address.

## Rust

```rust
let txid = "your_deposit_txid".to_string();
let vout = 0;
let destination_address = "bc1qexample...".to_string(); // Your Bitcoin address

// Set the fee for the refund transaction using the half-hour feerate
let recommended_fees = sdk.recommended_fees().await?;
let fee = Fee::Rate {
    sat_per_vbyte: recommended_fees.half_hour_fee,
};
// or using a fixed amount
//let fee = Fee::Fixed { amount: 500 };
//

let request = RefundDepositRequest {
    txid,
    vout,
    destination_address,
    fee,
};

let response = sdk.refund_deposit(request).await?;
info!("Refund transaction created:");
info!("Transaction ID: {}", response.tx_id);
info!("Transaction hex: {}", response.tx_hex);
```

## Swift

```swift
let txid = "your_deposit_txid"
let vout: UInt32 = 0
let destinationAddress = "bc1qexample..."  // Your Bitcoin address

// Set the fee for the refund transaction using the half-hour feerate
let recommendedFees = try await sdk.recommendedFees()
let fee = Fee.rate(satPerVbyte: recommendedFees.halfHourFee)
// or using a fixed amount
//let fee = Fee.fixed(amount: 500) // 500 sats
//

let request = RefundDepositRequest(
    txid: txid,
    vout: vout,
    destinationAddress: destinationAddress,
    fee: fee
)

let response = try await sdk.refundDeposit(request: request)
print("Refund transaction created:")
print("Transaction ID: \(response.txId)")
print("Transaction hex: \(response.txHex)")
```

## Kotlin

```kotlin
try {
    val txid = "your_deposit_txid"
    val vout = 0u
    val destinationAddress = "bc1qexample..." // Your Bitcoin address

    // Set the fee for the refund transaction using the half-hour feerate
    val recommendedFees = sdk.recommendedFees()
    val fee = Fee.Rate(recommendedFees.halfHourFee)
    // or using a fixed amount
    //val fee = Fee.Fixed(500u)
    //

    val request = RefundDepositRequest(
        txid = txid,
        vout = vout,
        destinationAddress = destinationAddress,
        fee = fee
    )

    val response = sdk.refundDeposit(request)
    // Log.v("Breez", "Refund transaction created:")
    // Log.v("Breez", "Transaction ID: ${response.txId}")
    // Log.v("Breez", "Transaction hex: ${response.txHex}")
} catch (e: Exception) {
    // handle error
}
```

## C#

```csharp
var txid = "your_deposit_txid";
var vout = 0U;
var destinationAddress = "bc1qexample...";  // Your Bitcoin address

// Set the fee for the refund transaction using the half-hour feerate
var recommendedFees = await sdk.RecommendedFees();
var fee = new Fee.Rate(satPerVbyte: recommendedFees.halfHourFee);
// or using a fixed amount
//var fee = new Fee.Fixed(amount: 500);
//

var request = new RefundDepositRequest(
    txid: txid,
    vout: vout,
    destinationAddress: destinationAddress,
    fee: fee
);

var response = await sdk.RefundDeposit(request: request);
Console.WriteLine("Refund transaction created:");
Console.WriteLine($"Transaction ID: {response.txId}");
Console.WriteLine($"Transaction hex: {response.txHex}");
```

## Javascript (Wasm)

```typescript
const txid = 'your_deposit_txid'
const vout = 0
const destinationAddress = 'bc1qexample...' // Your Bitcoin address

// Set the fee for the refund transaction using the half-hour feerate
const recommendedFees = await sdk.recommendedFees()
const fee: Fee = { type: 'rate', satPerVbyte: recommendedFees.halfHourFee }
// or using a fixed amount
// const fee: Fee = { type: 'fixed', amount: 500 }
//

const request: RefundDepositRequest = {
  txid,
  vout,
  destinationAddress,
  fee
}

const response = await sdk.refundDeposit(request)
console.log('Refund transaction created:')
console.log('Transaction ID:', response.txId)
console.log('Transaction hex:', response.txHex)
```

## React Native

```typescript
const txid = 'your_deposit_txid'
const vout = 0
const destinationAddress = 'bc1qexample...' // Your Bitcoin address

// Set the fee for the refund transaction using the half-hour feerate
const recommendedFees = await sdk.recommendedFees()
const fee = new Fee.Rate({ satPerVbyte: recommendedFees.halfHourFee })
// or using a fixed amount
// const fee = new Fee.Fixed({ amount: BigInt(500) })
//

const request: RefundDepositRequest = {
  txid,
  vout,
  destinationAddress,
  fee
}

const response = await sdk.refundDeposit(request)
console.log('Refund transaction created:')
console.log('Transaction ID:', response.txId)
console.log('Transaction hex:', response.txHex)
```

## Flutter

```dart
String txid = "your_deposit_txid";
int vout = 0;
String destinationAddress = "bc1qexample..."; // Your Bitcoin address

// Set the fee for the refund transaction using the half-hour feerate
final recommendedFees = await sdk.recommendedFees();
Fee fee = Fee.rate(satPerVbyte: recommendedFees.halfHourFee);
// or using a fixed amount
//Fee fee = Fee.fixed(amount: BigInt.from(500));
//

final request = RefundDepositRequest(
  txid: txid,
  vout: vout,
  destinationAddress: destinationAddress,
  fee: fee,
);

final response = await sdk.refundDeposit(request: request);
print("Refund transaction created:");
print("Transaction ID: ${response.txId}");
print("Transaction hex: ${response.txHex}");
```

## Python

```python
try:
    txid = "your_deposit_txid"
    vout = 0
    destination_address = "bc1qexample..."  # Your Bitcoin address

    # Set the fee for the refund transaction using the half-hour feerate
    recommended_fees = await sdk.recommended_fees()
    fee = Fee.RATE(sat_per_vbyte=recommended_fees.half_hour_fee)
    # or using a fixed amount
    #fee = Fee.FIXED(amount=500)
    #

    request = RefundDepositRequest(
        txid=txid, vout=vout, destination_address=destination_address, fee=fee
    )

    response = await sdk.refund_deposit(request=request)
    logging.info("Refund transaction created:")
    logging.info(f"Transaction ID: {response.tx_id}")
    logging.info(f"Transaction hex: {response.tx_hex}")
except Exception as error:
    logging.error(error)
    raise
```

## Go

```go
txid := "<your_deposit_txid>"
vout := uint32(0)
destinationAddress := "bc1qexample..." // Your Bitcoin address

// Set the fee for the refund transaction using the half-hour feerate
recommendedFees, err := sdk.RecommendedFees()
if err != nil {
	return err
}
fee := breez_sdk_spark.Fee(breez_sdk_spark.FeeRate{SatPerVbyte: recommendedFees.HalfHourFee})
// or using a fixed amount
//fee := breez_sdk_spark.Fee(breez_sdk_spark.FeeFixed{Amount: 500})
//

request := breez_sdk_spark.RefundDepositRequest{
	Txid:               txid,
	Vout:               vout,
	DestinationAddress: destinationAddress,
	Fee:                fee,
}
response, err := sdk.RefundDeposit(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}

log.Print("Refund transaction created:")
log.Printf("Transaction ID: %v", response.TxId)
log.Printf("Transaction hex: %v", response.TxHex)
```



- A claimed deposit cannot be refunded. Check that its `instant_claim_status` is not `InstantClaimStatus::Claimed` before offering a refund.
- A deposit can only be refunded once it has enough confirmations. Before that, `refund_deposit` fails and nothing is signed or stored, so retry after a few more blocks.
- The fee must cover at least 1 sat/vB of the refund transaction: around 99 sats to a native segwit address and 111 sats to a taproot one. A lower fee is rejected, and the error states the minimum.

[Recommended fees](#recommended-fees) help choose the fee.

## Tracking a refund

`refund_state` on the deposit reports the refund's progress:

- `RefundState::BroadcastPending`: signed and stored, but not yet seen on the network. The SDK rebroadcasts it on every sync, so a temporary network problem resolves on its own.
- `RefundState::Broadcast`: accepted by the network and waiting to confirm. The deposit leaves `list_unclaimed_deposits` once it confirms.

A refund paying close to the minimum can stay at `RefundState::BroadcastPending` if the network's minimum relay fee rises above what it pays, and rebroadcasting cannot fix that. Read `last_error` for the network's reason, then call `refund_deposit` again with a higher fee to replace it. The replacement has to pay more than the original, and a fee too low to replace it is rejected with the minimum in the error.

## Recommended fees

`recommended_fees` returns fee rate estimates for different confirmation targets.

### Rust

```rust
let response = sdk.recommended_fees().await?;
info!("Fastest fee: {} sats/vByte", response.fastest_fee);
info!("Half-hour fee: {} sats/vByte", response.half_hour_fee);
info!("Hour fee: {} sats/vByte", response.hour_fee);
info!("Economy fee: {} sats/vByte", response.economy_fee);
info!("Minimum fee: {} sats/vByte", response.minimum_fee);
```

### Swift

```swift
let response = try await sdk.recommendedFees()
print("Fastest fee: \(response.fastestFee) sats/vByte")
print("Half-hour fee: \(response.halfHourFee) sats/vByte")
print("Hour fee: \(response.hourFee) sats/vByte")
print("Economy fee: \(response.economyFee) sats/vByte")
print("Minimum fee: \(response.minimumFee) sats/vByte")
```

### Kotlin

```kotlin
val response = sdk.recommendedFees()
println("Fastest fee: ${response.fastestFee} sats/vByte")
println("Half-hour fee: ${response.halfHourFee} sats/vByte")
println("Hour fee: ${response.hourFee} sats/vByte")
println("Economy fee: ${response.economyFee} sats/vByte")
println("Minimum fee: ${response.minimumFee} sats/vByte")
```

### C#

```csharp
var response = await sdk.RecommendedFees();
    Console.WriteLine($"Fastest fee: {response.fastestFee} sats/vByte");
    Console.WriteLine($"Half-hour fee: {response.halfHourFee} sats/vByte");
    Console.WriteLine($"Hour fee: {response.hourFee} sats/vByte");
    Console.WriteLine($"Economy fee: {response.economyFee} sats/vByte");
    Console.WriteLine($"Minimum fee: {response.minimumFee} sats/vByte");
}
```

### Javascript (Wasm)

```typescript
const response = await sdk.recommendedFees()
console.log('Fastest fee:', response.fastestFee, 'sats/vByte')
console.log('Half-hour fee:', response.halfHourFee, 'sats/vByte')
console.log('Hour fee:', response.hourFee, 'sats/vByte')
console.log('Economy fee:', response.economyFee, 'sats/vByte')
console.log('Minimum fee:', response.minimumFee, 'sats/vByte')
```

### React Native

```typescript
const response = await sdk.recommendedFees()
console.log('Fastest fee:', response.fastestFee, 'sats/vByte')
console.log('Half-hour fee:', response.halfHourFee, 'sats/vByte')
console.log('Hour fee:', response.hourFee, 'sats/vByte')
console.log('Economy fee:', response.economyFee, 'sats/vByte')
console.log('Minimum fee:', response.minimumFee, 'sats/vByte')
```

### Flutter

```dart
final response = await sdk.recommendedFees();
print("Fastest fee: ${response.fastestFee} sats/vByte");
print("Half-hour fee: ${response.halfHourFee} sats/vByte");
print("Hour fee: ${response.hourFee} sats/vByte");
print("Economy fee: ${response.economyFee} sats/vByte");
print("Minimum fee: ${response.minimumFee} sats/vByte");
```

### Python

```python
response = await sdk.recommended_fees()
logging.info(f"Fastest fee: {response.fastest_fee} sats/vByte")
logging.info(f"Half-hour fee: {response.half_hour_fee} sats/vByte")
logging.info(f"Hour fee: {response.hour_fee} sats/vByte")
logging.info(f"Economy fee: {response.economy_fee} sats/vByte")
logging.info(f"Minimum fee: {response.minimum_fee} sats/vByte")
```

### Go

```go
response, err := sdk.RecommendedFees()
if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}
log.Printf("Fastest fee: %v sats/vByte", response.FastestFee)
log.Printf("Half-hour fee: %v sats/vByte", response.HalfHourFee)
log.Printf("Hour fee: %v sats/vByte", response.HourFee)
log.Printf("Economy fee: %v sats/vByte", response.EconomyFee)
log.Printf("Minimum fee: %v sats/vByte", response.MinimumFee)
```



---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
