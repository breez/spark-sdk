# Tracking deposits

## Listing deposits

`list_unclaimed_deposits` returns every deposit the SDK is tracking, from the moment it reaches the mempool until its output is spent by a claim or a refund.

### Rust

```rust
let request = ListUnclaimedDepositsRequest {};
let response = sdk.list_unclaimed_deposits(request).await?;

for deposit in response.deposits {
    info!("Unclaimed deposit: {}:{}", deposit.txid, deposit.vout);
    info!("Amount: {} sats", deposit.amount_sats);

    if let Some(claim_error) = &deposit.claim_error {
        match claim_error {
            DepositClaimError::MaxDepositClaimFeeExceeded {
                max_fee_sats,
                required_fee_sats,
                ..
            } => {
                let max_fee =
                    max_fee_sats.map_or("none".to_string(), |sats| format!("{sats} sats"));
                info!(
                    "Max claim fee exceeded. Max: {max_fee}, Required: {required_fee_sats} sats"
                );
            }
            DepositClaimError::MissingUtxo { .. } => {
                info!("UTXO not found when claiming deposit");
            }
            DepositClaimError::DepositTooSmall { .. } => {
                info!("Deposit too small to claim");
            }
            DepositClaimError::Generic { message } => {
                info!("Claim failed: {}", message);
            }
        }
    }
}
```

### Swift

```swift
let request = ListUnclaimedDepositsRequest()
let response = try await sdk.listUnclaimedDeposits(request: request)

for deposit in response.deposits {
    print("Unclaimed deposit: \(deposit.txid):\(deposit.vout)")
    print("Amount: \(deposit.amountSats) sats")

    if let claimError = deposit.claimError {
        switch claimError {
        case .maxDepositClaimFeeExceeded(let tx, let vout, let maxFeeSats, let requiredFeeSats):
            let maxFeeStr = maxFeeSats.map { "\($0) sats" } ?? "none"
            print(
                "Max claim fee exceeded. Max: \(maxFeeStr), "
                    + "Required: \(requiredFeeSats) sats"
            )
        case .missingUtxo(let tx, let vout):
            print("UTXO not found when claiming deposit")
        case .depositTooSmall(let tx, let vout):
            print("Deposit too small to claim")
        case .generic(let message):
            print("Claim failed: \(message)")
        }
    }
}
```

### Kotlin

```kotlin
try {
    val request = ListUnclaimedDepositsRequest
    val response = sdk.listUnclaimedDeposits(request)

    for (deposit in response.deposits) {
        // Log.v("Breez", "Unclaimed deposit: ${deposit.txid}:${deposit.vout}")
        // Log.v("Breez", "Amount: ${deposit.amountSats} sats")

        deposit.claimError?.let { claimError ->
            when (claimError) {
                is DepositClaimError.MaxDepositClaimFeeExceeded -> {
                    val maxFeeStr = claimError.maxFeeSats?.let { "$it sats" } ?: "none"
                    // Log.v("Breez", "Max claim fee exceeded. Max: $maxFeeStr,
                    // Required: ${claimError.requiredFeeSats} sats")
                }
                is DepositClaimError.MissingUtxo -> {
                    // Log.v("Breez", "UTXO not found when claiming deposit")
                }
                is DepositClaimError.DepositTooSmall -> {
                    // Log.v("Breez", "Deposit too small to claim")
                }
                is DepositClaimError.Generic -> {
                    // Log.v("Breez", "Claim failed: ${claimError.message}")
                }
            }
        }
    }
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
var request = new ListUnclaimedDepositsRequest();
var response = await sdk.ListUnclaimedDeposits(request: request);

foreach (var deposit in response.deposits)
{
    Console.WriteLine($"Unclaimed deposit: {deposit.txid}:{deposit.vout}");
    Console.WriteLine($"Amount: {deposit.amountSats} sats");

    if (deposit.claimError != null)
    {
        if (deposit.claimError is DepositClaimError.MaxDepositClaimFeeExceeded exceeded)
        {
            var maxFeeStr = exceeded.maxFeeSats is ulong sats ? $"{sats} sats" : "none";
            Console.WriteLine($"Max claim fee exceeded. Max: {maxFeeStr}, " +
                            $"Required: {exceeded.requiredFeeSats} sats");
        }
        else if (deposit.claimError is DepositClaimError.MissingUtxo)
        {
            Console.WriteLine("Claim failed: UTXO not found");
        }
        else if (deposit.claimError is DepositClaimError.DepositTooSmall)
        {
            Console.WriteLine("Claim failed: deposit too small to claim");
        }
        else if (deposit.claimError is DepositClaimError.Generic generic)
        {
            Console.WriteLine($"Claim failed: {generic.message}");
        }
    }
}
```

### Javascript (Wasm)

```typescript
const request: ListUnclaimedDepositsRequest = {}
const response = await sdk.listUnclaimedDeposits(request)

for (const deposit of response.deposits) {
  console.log(`Unclaimed deposit: ${deposit.txid}:${deposit.vout}`)
  console.log(`Amount: ${deposit.amountSats} sats`)

  if (deposit.claimError != null) {
    switch (deposit.claimError.type) {
      case 'maxDepositClaimFeeExceeded': {
        let maxFeeStr = 'none'
        if (deposit.claimError.maxFeeSats != null) {
          maxFeeStr = `${deposit.claimError.maxFeeSats} sats`
        }
        console.log(
          `Max claim fee exceeded. Max: ${maxFeeStr}, ` +
          `Required: ${deposit.claimError.requiredFeeSats} sats`
        )
        break
      }
      case 'missingUtxo':
        console.log('UTXO not found when claiming deposit')
        break
      case 'depositTooSmall':
        console.log('Deposit too small to claim')
        break
      case 'generic':
        console.log(`Claim failed: ${deposit.claimError.message}`)
        break
    }
  }
}
```

### React Native

```typescript
const request: ListUnclaimedDepositsRequest = {}
const response = await sdk.listUnclaimedDeposits(request)

for (const deposit of response.deposits) {
  console.log(`Unclaimed deposit: ${deposit.txid}:${deposit.vout}`)
  console.log(`Amount: ${deposit.amountSats} sats`)

  if (deposit.claimError != null) {
    if (deposit.claimError?.tag === DepositClaimError_Tags.MaxDepositClaimFeeExceeded) {
      let maxFeeStr = 'none'
      if (deposit.claimError.inner.maxFeeSats != null) {
        maxFeeStr = `${deposit.claimError.inner.maxFeeSats} sats`
      }
      console.log(
        `Max claim fee exceeded. Max: ${maxFeeStr}, ` +
        `Required: ${deposit.claimError.inner.requiredFeeSats} sats`
      )
    } else if (deposit.claimError?.tag === DepositClaimError_Tags.MissingUtxo) {
      console.log('UTXO not found when claiming deposit')
    } else if (deposit.claimError?.tag === DepositClaimError_Tags.DepositTooSmall) {
      console.log('Deposit too small to claim')
    } else if (deposit.claimError?.tag === DepositClaimError_Tags.Generic) {
      console.log(`Claim failed: ${deposit.claimError.inner.message}`)
    }
  }
}
```

### Flutter

```dart
final request = ListUnclaimedDepositsRequest();
final response = await sdk.listUnclaimedDeposits(request: request);

for (DepositInfo deposit in response.deposits) {
  print("Unclaimed deposit: ${deposit.txid}:${deposit.vout}");
  print("Amount: ${deposit.amountSats} sats");

  final claimError = deposit.claimError;
  if (claimError is DepositClaimError_MaxDepositClaimFeeExceeded) {
    final maxFee = claimError.maxFeeSats;
    final maxFeeStr = maxFee != null ? '$maxFee sats' : 'none';
    print("Max claim fee exceeded. Max: $maxFeeStr, "
        "Required: ${claimError.requiredFeeSats} sats");
  } else if (claimError is DepositClaimError_MissingUtxo) {
    print("UTXO not found when claiming deposit");
  } else if (claimError is DepositClaimError_DepositTooSmall) {
    print("Deposit too small to claim");
  } else if (claimError is DepositClaimError_Generic) {
    print("Claim failed: ${claimError.message}");
  }
}
```

### Python

```python
try:
    request = ListUnclaimedDepositsRequest()
    response = await sdk.list_unclaimed_deposits(request=request)

    for deposit in response.deposits:
        logging.info(f"Unclaimed deposit: {deposit.txid}:{deposit.vout}")
        logging.info(f"Amount: {deposit.amount_sats} sats")

        if deposit.claim_error:
            if isinstance(
                deposit.claim_error, DepositClaimError.MAX_DEPOSIT_CLAIM_FEE_EXCEEDED
            ):
                max_fee_sats = deposit.claim_error.max_fee_sats
                max_fee_str = "none" if max_fee_sats is None else f"{max_fee_sats} sats"
                logging.info(
                    f"Max claim fee exceeded. Max: {max_fee_str}, "
                    f"Required: {deposit.claim_error.required_fee_sats} sats"
                )
            elif isinstance(deposit.claim_error, DepositClaimError.MISSING_UTXO):
                logging.info("Claim failed: UTXO not found")
            elif isinstance(deposit.claim_error, DepositClaimError.DEPOSIT_TOO_SMALL):
                logging.info("Claim failed: deposit too small to claim")
            elif isinstance(deposit.claim_error, DepositClaimError.GENERIC):
                logging.info(f"Claim failed: {deposit.claim_error.message}")
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
request := breez_sdk_spark.ListUnclaimedDepositsRequest{}
response, err := sdk.ListUnclaimedDeposits(request)

if err != nil {
	var sdkErr *breez_sdk_spark.SdkError
	if errors.As(err, &sdkErr) {
		// Handle SdkError - can inspect specific variants if needed
		// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
	}
	return err
}

for _, deposit := range response.Deposits {
	log.Printf("Unclaimed Deposit: %v:%v", deposit.Txid, deposit.Vout)
	log.Printf("Amount: %v sats", deposit.AmountSats)

	if claimErr := *deposit.ClaimError; claimErr != nil {
		switch claimErr := claimErr.(type) {
		case breez_sdk_spark.DepositClaimErrorMaxDepositClaimFeeExceeded:
			maxFeeStr := "none"
			if claimErr.MaxFeeSats != nil {
				maxFeeStr = fmt.Sprintf("%v sats", *claimErr.MaxFeeSats)
			}
			log.Printf(
				"Max claim fee exceeded. Max: %v, Required: %v sats",
				maxFeeStr,
				claimErr.RequiredFeeSats,
			)
		case breez_sdk_spark.DepositClaimErrorMissingUtxo:
			log.Print("UTXO not found when claiming deposit")
		case breez_sdk_spark.DepositClaimErrorDepositTooSmall:
			log.Print("Deposit too small to claim")
		case breez_sdk_spark.DepositClaimErrorGeneric:
			log.Printf("Claim failed: %v", claimErr.Message)
		}
	}
}
```



## Deposit states

What to show for a deposit follows from a few of its fields. The first row that matches applies.

| State | How to tell | What to show |
|---|---|---|
| Claimed | `instant_claim_status` is `InstantClaimStatus::Claimed` | Settled. No action needed. |
| Being claimed | `instant_claim_status` is `InstantClaimStatus::Submitted` | Being credited |
| Refunding | `refund_state` is set | The refund's progress. See [Tracking a refund](onchain_refunds.md#tracking-a-refund). |
| Claim failed | `claim_error` is set | The reason, with the option to [claim manually](onchain_manual_claims.md) or [refund](onchain_refunds.md) |
| Pending | None of the above | Pending. The SDK claims it automatically. |

A claimed deposit stays in the list until the provider spends its output, some time after the credit. When the SDK claims a deposit on its own it emits `SdkEvent::ClaimedDeposits` as the claim is submitted, so a deposit can appear in that event and in the list at the same time.

A deposit claimed by another instance sharing the wallet, or on another device, becomes `InstantClaimStatus::Claimed` the next time the SDK tries to claim it. No `SdkEvent::ClaimedDeposits` event is emitted for it, since the claim was not made here, but the credit still arrives as a payment.

## Seeing deposits before they confirm

A deposit arrives through `SdkEvent::NewDeposits` and appears in `list_unclaimed_deposits` as soon as it reaches the mempool, with `is_mature` false until it reaches the standard claim depth.

**Developer note**

The Spark operators only report confirmed deposits, so the SDK finds unconfirmed ones by asking its chain service about recently used deposit addresses, one request per address per sync. Requesting a receive address watches it for 24 hours, and requesting it again restarts that window. An address that received a deposit stays watched until the deposit confirms.

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
