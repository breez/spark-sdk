# Claiming on-chain deposits

On-chain deposits go through three stages. Once detected, the deposit is visible in the SDK and each deposit includes a `is_mature` field. After **3 on-chain confirmations** the deposit has sufficient confirmations (`is_mature` is true) and the SDK [automatically attempts](#setting-a-max-fee-for-automatic-claims) to claim it. The SDK also claims automatically [before maturity](#claiming-before-maturity) when the configured ceiling covers the provider's spread, so a deposit can be credited sooner than 3 confirmations. If the maximum deposit claim fee is too low for either, the deposit won't be automatically claimed and should be [manually claimed](#manually-claiming-deposits).

## Setting a max fee for automatic claims

The [maximum deposit claim fee](config.md#max-deposit-claim-fee) setting in the SDK configuration defines the maximum fee the SDK uses when automatically claiming an on-chain deposit. The SDK's default fee limit is set to 1 sats/vbyte, which is low and requires manual claiming when fees exceed this threshold. You can set a higher fee, either in sats/vbyte, in absolute sats, or to the fastest recommended fee at the time of claim, with a leeway in sats/vbyte.

This ceiling is not only an on-chain fee tolerance. It also caps what the provider may take to credit a deposit [before it matures](#claiming-before-maturity), so the value you choose decides both how much on-chain fee the SDK will pay and whether deposits are claimed early at all.

To increase the likelihood of automatically claiming deposits, you may set the maximum fee to the fastest recommended rate at the time of claim, which can result in higher fees.

### Rust

```rust
// Create the default config
let mut config = default_config(Network::Mainnet);
config.api_key = Some("<breez api key>".to_string());

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
config.max_deposit_claim_fee = Some(MaxFee::NetworkRecommended {
    leeway_sat_per_vbyte: 1,
});
```

### Swift

```swift
// Create the default config
var config = defaultConfig(network: Network.mainnet)
config.apiKey = "<breez api key>"

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
config.maxDepositClaimFee = MaxFee.networkRecommended(leewaySatPerVbyte: 1)
```

### Kotlin

```kotlin
// Create the default config
val config = defaultConfig(Network.MAINNET)
config.apiKey = "<breez api key>"

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
config.maxDepositClaimFee = MaxFee.NetworkRecommended(leewaySatPerVbyte = 1u)
```

### C#

```csharp
// Create the default config
var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
{
    apiKey = "<breez api key>"
};

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
config = config with { maxDepositClaimFee = new MaxFee.NetworkRecommended(leewaySatPerVbyte: 1) };
```

### Javascript (Wasm)

```typescript
// Create the default config
const config = defaultConfig('mainnet')
config.apiKey = '<breez api key>'

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
config.maxDepositClaimFee = { type: 'networkRecommended', leewaySatPerVbyte: 1 }
```

### React Native

```typescript
// Create the default config
const config = defaultConfig(Network.Mainnet)
config.apiKey = '<breez api key>'

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
config.maxDepositClaimFee = new MaxFee.NetworkRecommended({ leewaySatPerVbyte: BigInt(1) })
```

### Flutter

```dart
// Create the default config
var config = defaultConfig(network: Network.mainnet);
config = config.copyWith(apiKey: "<breez api key>");

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
config = config.copyWith(
    maxDepositClaimFee:
        MaxFee.networkRecommended(leewaySatPerVbyte: BigInt.from(1)));
```

### Python

```python
# Create the default config
config = default_config(network=Network.MAINNET)
config.api_key = "<breez api key>"

# Set the maximum fee to the fastest network recommended fee at the time of claim
# with a leeway of 1 sats/vbyte
config.max_deposit_claim_fee = MaxFee.NETWORK_RECOMMENDED(leeway_sat_per_vbyte=1)
```

### Go

```go
// Create the default config
config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
apiKey := "<breez api key>"
config.ApiKey = &apiKey

// Set the maximum fee to the fastest network recommended fee at the time of claim
// with a leeway of 1 sats/vbyte
networkRecommendedInterface := breez_sdk_spark.MaxFee(
	breez_sdk_spark.MaxFeeNetworkRecommended{LeewaySatPerVbyte: 1},
)
config.MaxDepositClaimFee = &networkRecommendedInterface
```



However, even when setting a high fee, the SDK might still fail to automatically claim deposits. In these cases, it's recommended to manually claim them by letting the end user accept the required fees. When [manual intervention](#manually-claiming-deposits) is required, the SDK emits an `SdkEvent::UnclaimedDeposits` event containing information about the deposit. See [Listening to events](events.md) for how to subscribe to events.

## Claiming before maturity

A deposit does not have to wait for maturity. The Spark Service Provider will front the credited amount earlier and take a spread for carrying the risk, and the SDK claims this way automatically whenever the spread fits within the [maximum deposit claim fee](config.md#max-deposit-claim-fee). The default of 1 sat/vbyte works out to about 99 sats, below any spread the provider charges, so deposits are claimed at maturity until the ceiling is raised enough to cover one. The same applies to `claim_deposit`, which claims a not-yet-mature deposit early when its own `max_fee` allows.

The spread is largely the on-chain cost of the provider's claim plus a percentage of the deposit, so it grows with the deposit.

## Manually claiming deposits

When a deposit cannot be automatically claimed due to the configured maximum fee being too low, you can manually claim it by specifying a higher fee limit. The recommended approach is to display a user interface showing the required fee amount and request user approval before proceeding with manual claiming.

Claiming a deposit the SDK is already claiming, whether from a background attempt or another call, returns `SdkError::DepositClaimInProgress`. The claim already running may still succeed, so treat this as transient rather than as a failure to show the user.

### Rust

```rust
if let Some(DepositClaimError::MaxDepositClaimFeeExceeded {
    required_fee_sats, ..
}) = &deposit.claim_error
{
    // Show UI to user with the required fee and get approval
    let user_approved = true; // Replace with actual user approval logic

    if user_approved {
        let request = ClaimDepositRequest {
            txid: deposit.txid.clone(),
            vout: deposit.vout,
            max_fee: Some(MaxFee::Fixed {
                amount: *required_fee_sats,
            }),
        };
        sdk.claim_deposit(request).await?;
    }
}
```

### Swift

```swift
if case .maxDepositClaimFeeExceeded(_, _, _, let requiredFeeSats, _) = deposit.claimError {
    // Show UI to user with the required fee and get approval
    let userApproved = true  // Replace with actual user approval logic

    if userApproved {
        let claimRequest = ClaimDepositRequest(
            txid: deposit.txid,
            vout: deposit.vout,
            maxFee: MaxFee.fixed(amount: requiredFeeSats)
        )
        try await sdk.claimDeposit(request: claimRequest)
    }
}
```

### Kotlin

```kotlin
try {
    val claimError = deposit.claimError
    if (claimError is DepositClaimError.MaxDepositClaimFeeExceeded) {
        val requiredFee = claimError.requiredFeeSats

        // Show UI to user with the required fee and get approval
        val userApproved = true // Replace with actual user approval logic

        if (userApproved) {
            val claimRequest = ClaimDepositRequest(
                txid = deposit.txid,
                vout = deposit.vout,
                maxFee = MaxFee.Fixed(requiredFee)
            )
            sdk.claimDeposit(claimRequest)
        }
    }
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
if (deposit.claimError is DepositClaimError.MaxDepositClaimFeeExceeded exceeded)
{
    var requiredFee = exceeded.requiredFeeSats;

    // Show UI to user with the required fee and get approval
    var userApproved = true; // Replace with actual user approval logic

    if (userApproved)
    {
        var claimRequest = new ClaimDepositRequest(
            txid: deposit.txid,
            vout: deposit.vout,
            maxFee: new MaxFee.Fixed(amount: requiredFee)
        );
        await sdk.ClaimDeposit(request: claimRequest);
    }
}
```

### Javascript (Wasm)

```typescript
if (deposit.claimError?.type === 'maxDepositClaimFeeExceeded') {
  const requiredFee = deposit.claimError.requiredFeeSats

  // Show UI to user with the required fee and get approval
  const userApproved = true // Replace with actual user approval logic

  if (userApproved) {
    const claimRequest: ClaimDepositRequest = {
      txid: deposit.txid,
      vout: deposit.vout,
      maxFee: { type: 'fixed', amount: requiredFee }
    }
    await sdk.claimDeposit(claimRequest)
  }
}
```

### React Native

```typescript
if (deposit.claimError?.tag === DepositClaimError_Tags.MaxDepositClaimFeeExceeded) {
  const requiredFee = deposit.claimError.inner.requiredFeeSats

  // Show UI to user with the required fee and get approval
  const userApproved = true // Replace with actual user approval logic

  if (userApproved) {
    const claimRequest: ClaimDepositRequest = {
      txid: deposit.txid,
      vout: deposit.vout,
      maxFee: new MaxFee.Fixed({ amount: requiredFee })
    }
    await sdk.claimDeposit(claimRequest)
  }
}
```

### Flutter

```dart
final claimError = deposit.claimError;
if (claimError is DepositClaimError_MaxDepositClaimFeeExceeded) {
  final requiredFee = claimError.requiredFeeSats;

  // Show UI to user with the required fee and get approval
  bool userApproved = true; // Replace with actual user approval logic

  if (userApproved) {
    final claimRequest = ClaimDepositRequest(
      txid: deposit.txid,
      vout: deposit.vout,
      maxFee: MaxFee.fixed(amount: requiredFee),
    );
    await sdk.claimDeposit(request: claimRequest);
  }
}
```

### Python

```python
try:
    if isinstance(
        deposit.claim_error, DepositClaimError.MAX_DEPOSIT_CLAIM_FEE_EXCEEDED
    ):
        required_fee = deposit.claim_error.required_fee_sats

        # Show UI to user with the required fee and get approval
        user_approved = True  # Replace with actual user approval logic

        if user_approved:
            claim_request = ClaimDepositRequest(
                txid=deposit.txid,
                vout=deposit.vout,
                max_fee=Fee.FIXED(amount=required_fee),
            )
            await sdk.claim_deposit(request=claim_request)
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
if claimErr := *deposit.ClaimError; claimErr != nil {
	if exceeded, ok := claimErr.(breez_sdk_spark.DepositClaimErrorMaxDepositClaimFeeExceeded); ok {
		requiredFee := exceeded.RequiredFeeSats

		// Show UI to user with the required fee and get approval
		userApproved := true // Replace with actual user approval logic

		if userApproved {
			maxFee := breez_sdk_spark.MaxFee(breez_sdk_spark.MaxFeeFixed{Amount: requiredFee})
			claimRequest := breez_sdk_spark.ClaimDepositRequest{
				Txid:   deposit.Txid,
				Vout:   deposit.Vout,
				MaxFee: &maxFee,
			}
			_, err := sdk.ClaimDeposit(claimRequest)
			if err != nil {
				var sdkErr *breez_sdk_spark.SdkError
				if errors.As(err, &sdkErr) {
					// Handle SdkError - can inspect specific variants if needed
					// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
				}
				return err
			}
		}
	}
}
```



### Showing the choice to the user

`fetch_claim_deposit_quote` prices both ways of claiming a deposit, so an app can offer the choice rather than deciding for the user. It returns the deposit's current `confirmations` alongside a quote for claiming early and one for claiming at maturity, each carrying the fee and the `confirmations_required`, which is the depth it becomes claimable at rather than a count of blocks still to wait. Subtract the deposit's current confirmations for that: an early claim claimable at 1 confirmation, on a deposit with 0, is available a block from now.

The early quote is absent when the provider will not front this particular deposit, and when claiming early would not actually be earlier: once a deposit has matured, or when the provider would only credit at maturity's own depth, waiting is both cheaper and no slower, so there is no choice left to offer. The early quote is priced whether or not the configured [maximum deposit claim fee](config.md#max-deposit-claim-fee) would allow it, since that ceiling is usually far below a spread and the point of the quote is to let the user decide. Acting on it therefore means passing a `max_fee` to `claim_deposit` of at least the quoted `fee_sats`; with a lower one the call returns `SdkError::MaxDepositClaimFeeExceeded` and the deposit waits for maturity instead. The quote for maturity is always present, but may be flagged `is_estimate` when the provider will not quote a deposit this early, in which case the fee is derived from current on-chain fees and the final one may differ.

A claim made before maturity settles asynchronously, so `claim_deposit` returns no payment. Watch for it via `list_payments` or the [payment events](events.md).

#### Rust

```rust
let quote = sdk
    .fetch_claim_deposit_quote(FetchClaimDepositQuoteRequest {
        txid: deposit.txid.clone(),
        vout: deposit.vout,
    })
    .await?;

// Claiming once the deposit matures, and how many blocks that is away.
let blocks_to_wait = quote
    .mature
    .confirmations_required
    .saturating_sub(quote.confirmations);
info!(
    "Wait {} blocks and pay {} sats",
    blocks_to_wait, quote.mature.fee_sats
);

// Claiming earlier, when the provider offers it.
if let Some(instant) = &quote.instant {
    let blocks_to_wait = instant
        .confirmations_required
        .saturating_sub(quote.confirmations);
    info!(
        "Or wait {} blocks and pay {} sats",
        blocks_to_wait, instant.fee_sats
    );
}
```

#### Swift

```swift
let request = FetchClaimDepositQuoteRequest(txid: deposit.txid, vout: deposit.vout)
let quote = try await sdk.fetchClaimDepositQuote(request: request)

// Claiming once the deposit matures, and how many blocks that is away.
var blocksToWait: UInt32 = 0
if quote.mature.confirmationsRequired > quote.confirmations {
    blocksToWait = quote.mature.confirmationsRequired - quote.confirmations
}
print("Wait \(blocksToWait) blocks and pay \(quote.mature.feeSats) sats")

// Claiming earlier, when the provider offers it.
if let instant = quote.instant {
    var instantBlocks: UInt32 = 0
    if instant.confirmationsRequired > quote.confirmations {
        instantBlocks = instant.confirmationsRequired - quote.confirmations
    }
    print("Or wait \(instantBlocks) blocks and pay \(instant.feeSats) sats")
}
```

#### Kotlin

```kotlin
try {
    val quote = sdk.fetchClaimDepositQuote(
        FetchClaimDepositQuoteRequest(
            txid = deposit.txid,
            vout = deposit.vout
        )
    )

    // UInt subtraction wraps, so clamp the wait at zero.
    fun blocksToWait(required: UInt) =
        if (required > quote.confirmations) required - quote.confirmations else 0u

    // Claiming once the deposit matures, and how many blocks that is away.
    val matureWait = blocksToWait(quote.mature.confirmationsRequired)
    // Log.v("Breez", "Wait $matureWait blocks and pay ${quote.mature.feeSats} sats")

    // Claiming earlier, when the provider offers it.
    quote.instant?.let { instant ->
        val instantWait = blocksToWait(instant.confirmationsRequired)
        // Log.v("Breez", "Or wait $instantWait blocks and pay ${instant.feeSats} sats")
    }
} catch (e: Exception) {
    // handle error
}
```

#### C#

```csharp
var request = new FetchClaimDepositQuoteRequest(
    txid: deposit.txid,
    vout: deposit.vout
);
var quote = await sdk.FetchClaimDepositQuote(request: request);

// Claiming once the deposit matures, and how many blocks that is away.
var blocksToWait = quote.mature.confirmationsRequired > quote.confirmations
    ? quote.mature.confirmationsRequired - quote.confirmations
    : 0U;
Console.WriteLine($"Wait {blocksToWait} blocks and pay {quote.mature.feeSats} sats");

// Claiming earlier, when the provider offers it.
if (quote.instant is ClaimDepositQuote instant)
{
    var instantBlocksToWait = instant.confirmationsRequired > quote.confirmations
        ? instant.confirmationsRequired - quote.confirmations
        : 0U;
    Console.WriteLine($"Or wait {instantBlocksToWait} blocks and " +
                      $"pay {instant.feeSats} sats");
}
```

#### Javascript (Wasm)

```typescript
const quote = await sdk.fetchClaimDepositQuote({
  txid: deposit.txid,
  vout: deposit.vout
})

// Claiming once the deposit matures, and how many blocks that is away.
const blocksToWait = Math.max(
  0,
  quote.mature.confirmationsRequired - quote.confirmations
)
console.log(`Wait ${blocksToWait} blocks and pay ${quote.mature.feeSats} sats`)

// Claiming earlier, when the provider offers it.
if (quote.instant != null) {
  const instantBlocksToWait = Math.max(
    0,
    quote.instant.confirmationsRequired - quote.confirmations
  )
  console.log(
    `Or wait ${instantBlocksToWait} blocks and ` +
    `pay ${quote.instant.feeSats} sats`
  )
}
```

#### React Native

```typescript
const quote = await sdk.fetchClaimDepositQuote({
  txid: deposit.txid,
  vout: deposit.vout
})

// Claiming once the deposit matures, and how many blocks that is away.
const blocksToWait = Math.max(0, quote.mature.confirmationsRequired - quote.confirmations)
console.log(`Wait ${blocksToWait} blocks and pay ${quote.mature.feeSats} sats`)

// Claiming earlier, when the provider offers it.
const instant = quote.instant
if (instant != null) {
  const instantBlocks = Math.max(0, instant.confirmationsRequired - quote.confirmations)
  console.log(`Or wait ${instantBlocks} blocks and pay ${instant.feeSats} sats`)
}
```

#### Flutter

```dart
final request = FetchClaimDepositQuoteRequest(
  txid: deposit.txid,
  vout: deposit.vout,
);
final quote = await sdk.fetchClaimDepositQuote(request: request);

// Claiming once the deposit matures, and how many blocks that is away.
final confirmations = quote.confirmations;
final matureBlocks = quote.mature.confirmationsRequired > confirmations
    ? quote.mature.confirmationsRequired - confirmations
    : 0;
print("Wait $matureBlocks blocks and pay ${quote.mature.feeSats} sats");

// Claiming earlier, when the provider offers it.
final instant = quote.instant;
if (instant != null) {
  final instantBlocks = instant.confirmationsRequired > confirmations
      ? instant.confirmationsRequired - confirmations
      : 0;
  print("Or wait $instantBlocks blocks and pay ${instant.feeSats} sats");
}
```

#### Python

```python
try:
    request = FetchClaimDepositQuoteRequest(txid=deposit.txid, vout=deposit.vout)
    quote = await sdk.fetch_claim_deposit_quote(request=request)

    # Claiming once the deposit matures, and how many blocks that is away.
    blocks_to_wait = max(
        0, quote.mature.confirmations_required - quote.confirmations
    )
    logging.info(
        f"Wait {blocks_to_wait} blocks and pay {quote.mature.fee_sats} sats"
    )

    # Claiming earlier, when the provider offers it.
    if quote.instant is not None:
        blocks_to_wait = max(
            0, quote.instant.confirmations_required - quote.confirmations
        )
        logging.info(
            f"Or wait {blocks_to_wait} blocks and pay {quote.instant.fee_sats} sats"
        )
except Exception as error:
    logging.error(error)
    raise
```

#### Go

```go
quote, err := sdk.FetchClaimDepositQuote(breez_sdk_spark.FetchClaimDepositQuoteRequest{
	Txid: deposit.Txid,
	Vout: deposit.Vout,
})
if err != nil {
	return err
}

// Claiming once the deposit matures, and how many blocks that is away.
blocksToWait := uint32(0)
if quote.Mature.ConfirmationsRequired > quote.Confirmations {
	blocksToWait = quote.Mature.ConfirmationsRequired - quote.Confirmations
}
log.Printf("Wait %v blocks and pay %v sats", blocksToWait, quote.Mature.FeeSats)

// Claiming earlier, when the provider offers it.
if quote.Instant != nil {
	instantBlocks := uint32(0)
	if quote.Instant.ConfirmationsRequired > quote.Confirmations {
		instantBlocks = quote.Instant.ConfirmationsRequired - quote.Confirmations
	}
	log.Printf("Or wait %v blocks and pay %v sats", instantBlocks, quote.Instant.FeeSats)
}
```



## Listing unclaimed deposits

Retrieve all deposits that have not yet been claimed. This includes pending deposits that do not yet have sufficient confirmations, as well as deposits with sufficient confirmations that failed to claim (with the specific failure reason). Pending deposits will be automatically claimed once they have sufficient confirmations, or sooner if the configured ceiling covers an early claim.

A deposit claimed before maturity stays in the list with its `instant_claim_status` set to `InstantClaimStatus::Submitted` for a short time after submission, and is removed once the claim settles. When the SDK claims automatically it emits `SdkEvent::ClaimedDeposits` at submission, so a deposit can briefly appear both in that event and in this list.

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
                max_fee,
                required_fee_sats,
                required_fee_rate_sat_per_vbyte,
                ..
            } => {
                info!(
                    "Max claim fee exceeded. Max: {:?}, Required: {} sats or {} sats/vByte",
                    max_fee, required_fee_sats, required_fee_rate_sat_per_vbyte
                );
            }
            DepositClaimError::MissingUtxo { .. } => {
                info!("UTXO not found when claiming deposit");
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
        case .maxDepositClaimFeeExceeded(
            let tx, let vout, let maxFee, let requiredFeeSats, let requiredFeeRateSatPerVbyte):
            let maxFeeStr: String
            if let maxFee = maxFee {
                switch maxFee {
                case .fixed(let amount):
                    maxFeeStr = "\(amount) sats"
                case .rate(let satPerVbyte):
                    maxFeeStr = "\(satPerVbyte) sats/vByte"
                }
            } else {
                maxFeeStr = "none"
            }
            print(
                "Max claim fee exceeded. Max: \(maxFeeStr), "
                    + "Required: \(requiredFeeSats) sats or "
                    + "\(requiredFeeRateSatPerVbyte) sats/vByte"
            )
        case .missingUtxo(let tx, let vout):
            print("UTXO not found when claiming deposit")
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
                    val maxFee = claimError.maxFee
                    val maxFeeStr = when (maxFee) {
                        is Fee.Fixed -> "${maxFee.amount} sats"
                        is Fee.Rate -> "${maxFee.satPerVbyte} sats/vByte"
                        null -> "none"
                    }
                    // Log.v("Breez", "Max claim fee exceeded. Max: $maxFeeStr,
                    // Required: ${claimError.requiredFeeSats} sats or
                    // ${claimError.requiredFeeRateSatPerVbyte} sats/vByte")
                }
                is DepositClaimError.MissingUtxo -> {
                    // Log.v("Breez", "UTXO not found when claiming deposit")
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
            var maxFeeStr = "none";
            if (exceeded.maxFee != null)
            {
                if (exceeded.maxFee is Fee.Fixed fixedFee)
                {
                    maxFeeStr = $"{fixedFee.amount} sats";
                }
                else if (exceeded.maxFee is Fee.Rate rateFee)
                {
                    maxFeeStr = $"{rateFee.satPerVbyte} sats/vByte";
                }
            }
            Console.WriteLine($"Claim failed: Fee exceeded. Max: {maxFeeStr}, " +
                            $"Required: {exceeded.requiredFeeSats} sats or " +
                            $"{exceeded.requiredFeeRateSatPerVbyte} sats/vByte");
        }
        else if (deposit.claimError is DepositClaimError.MissingUtxo)
        {
            Console.WriteLine("Claim failed: UTXO not found");
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
        if (deposit.claimError.maxFee != null) {
          if (deposit.claimError.maxFee.type === 'fixed') {
            maxFeeStr = `${deposit.claimError.maxFee.amount} sats`
          } else if (deposit.claimError.maxFee.type === 'rate') {
            maxFeeStr = `${deposit.claimError.maxFee.satPerVbyte} sats/vByte`
          }
        }
        console.log(
          `Max claim fee exceeded. Max: ${maxFeeStr}, ` +
          `Required: ${deposit.claimError.requiredFeeSats} sats or ` +
          `${deposit.claimError.requiredFeeRateSatPerVbyte} sats/vByte`
        )
        break
      }
      case 'missingUtxo':
        console.log('UTXO not found when claiming deposit')
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
      if (deposit.claimError.inner.maxFee != null) {
        if (deposit.claimError.inner.maxFee.tag === Fee_Tags.Fixed) {
          maxFeeStr = `${deposit.claimError.inner.maxFee.inner.amount} sats`
        } else if (deposit.claimError.inner.maxFee.tag === Fee_Tags.Rate) {
          maxFeeStr = `${deposit.claimError.inner.maxFee.inner.satPerVbyte} sats/vByte`
        }
      }
      console.log(
        `Max claim fee exceeded. Max: ${maxFeeStr}, 
        Required: ${deposit.claimError.inner.requiredFeeSats} sats 
        or ${deposit.claimError.inner.requiredFeeRateSatPerVbyte} sats/vByte`
      )
    } else if (deposit.claimError?.tag === DepositClaimError_Tags.MissingUtxo) {
      console.log('UTXO not found when claiming deposit')
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
    final maxFeeStr = claimError.maxFee != null
        ? (claimError.maxFee is Fee_Fixed
            ? '${(claimError.maxFee as Fee_Fixed).amount} sats'
            : '${(claimError.maxFee as Fee_Rate).satPerVbyte} sats/vByte')
        : 'none';
    print("Max claim fee exceeded. Max: $maxFeeStr, "
        "Required: ${claimError.requiredFeeSats} sats or "
        "${claimError.requiredFeeRateSatPerVbyte} sats/vByte");
  } else if (claimError is DepositClaimError_MissingUtxo) {
    print("UTXO not found when claiming deposit");
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
                max_fee_str = "none"
                if deposit.claim_error.max_fee is not None:
                    if isinstance(deposit.claim_error.max_fee, Fee.FIXED):
                        max_fee_str = f"{deposit.claim_error.max_fee.amount} sats"
                    elif isinstance(deposit.claim_error.max_fee, Fee.RATE):
                        max_fee_str = f"{deposit.claim_error.max_fee.sat_per_vbyte} sats/vByte"
                logging.info(
                    f"Claim failed: Fee exceeded. Max: {max_fee_str}, "
                    f"Required: {deposit.claim_error.required_fee_sats} sats "
                    f"or {deposit.claim_error.required_fee_rate_sat_per_vbyte} sats/vByte"
                )
            elif isinstance(deposit.claim_error, DepositClaimError.MISSING_UTXO):
                logging.info("Claim failed: UTXO not found")
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
			if claimErr.MaxFee != nil {
				switch fee := (*claimErr.MaxFee).(type) {
				case breez_sdk_spark.FeeFixed:
					maxFeeStr = fmt.Sprintf("%v sats", fee.Amount)
				case breez_sdk_spark.FeeRate:
					maxFeeStr = fmt.Sprintf("%v sats/vByte", fee.SatPerVbyte)
				}
			}
			log.Printf(
				"Max claim fee exceeded. Max: %v, Required: %v sats or %v sats/vByte",
				maxFeeStr,
				claimErr.RequiredFeeSats,
				claimErr.RequiredFeeRateSatPerVbyte,
			)
		case breez_sdk_spark.DepositClaimErrorMissingUtxo:
			log.Print("UTXO not found when claiming deposit")
		case breez_sdk_spark.DepositClaimErrorGeneric:
			log.Printf("Claim failed: %v", claimErr.Message)
		}
	}
}
```



## Refunding deposits

When a deposit cannot be successfully claimed you can refund it to an external Bitcoin address. This creates a transaction that sends the amount (minus transaction fees) to the specified destination address.

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for refund transactions.

A deposit can only be refunded once it has enough confirmations. Calling `refund_deposit` earlier fails, reporting the deposit as unknown while it is unconfirmed and as having too few confirmations for a block or so after that. Nothing is signed or stored when this happens, so retry after a few more blocks.

### Rust

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

### Swift

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

### Kotlin

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

### C#

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

### Javascript (Wasm)

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

### React Native

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

### Flutter

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

### Python

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

### Go

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



**Developer note**

The total fee must cover at least 1 sat/vB of the refund transaction so it can be relayed by the Bitcoin network. The exact minimum depends on the size of the transaction, which varies with the destination address type: around 99 sats to a native segwit address and 111 sats to a taproot one. If the fee is lower, the refund request is rejected and the error states the required minimum.

### Tracking a refund

`refund_state` on `DepositInfo` reports how far the refund has got:

- **`RefundState::BroadcastPending`**: the refund is signed and stored but has not been seen on the network. The SDK rebroadcasts it on every sync until the deposit is spent, so a refund that failed to send because of a temporary network problem recovers on its own.
- **`RefundState::Broadcast`**: the network has accepted the refund and it is waiting to confirm. The deposit disappears from `list_unclaimed_deposits` once it does.

A refund created near the 1 sat/vB minimum can stay at `RefundState::BroadcastPending` indefinitely if the network's minimum relay fee later rises above what it pays. Rebroadcasting cannot fix this, because the network keeps refusing the same transaction. Read `last_error` for the reason the network gave, then call `refund_deposit` again at a higher fee to replace it.

Replacing a refund that is already on the network costs more than the original fee, because the replacement also pays to relay its own size. When the fee offered is too low, the call is rejected and the error states the minimum required.

## Implementing a custom claim logic

For advanced use cases, you may want to implement a custom claim logic instead of relying on the SDK's automatic process. This gives you complete control over when and how deposits are claimed.

To disable automatic claims, unset the [maximum deposit claim fee](config.md#max-deposit-claim-fee). Then use the methods described above to manually claim deposits based on your business logic.

Common scenarios for custom claiming logic include:

- **Dynamic fee adjustment**: Adjust claiming fees based on market conditions or priority
- **Conditional claiming**: Only claim deposits that meet certain criteria (amount thresholds, time windows, etc.)
- **Integration with external systems**: Coordinate claims with other business processes

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for claiming deposits. For example, you can implement a custom claim logic to only claim deposits if the required fee rate is less than the fastest recommended fee (or any other).

### Rust

```rust
if let Some(DepositClaimError::MaxDepositClaimFeeExceeded {
    required_fee_rate_sat_per_vbyte,
    ..
}) = &deposit.claim_error
{
    let recommended_fees = sdk.recommended_fees().await?;

    if *required_fee_rate_sat_per_vbyte <= recommended_fees.fastest_fee {
        let request = ClaimDepositRequest {
            txid: deposit.txid.clone(),
            vout: deposit.vout,
            max_fee: Some(MaxFee::Rate {
                sat_per_vbyte: *required_fee_rate_sat_per_vbyte,
            }),
        };
        sdk.claim_deposit(request).await?;
    }
}
```

### Swift

```swift
if case .maxDepositClaimFeeExceeded(_, _, _, _, let requiredFeeRateSatPerVbyte) =
    deposit.claimError
{
    let recommendedFees = try await sdk.recommendedFees()

    if requiredFeeRateSatPerVbyte <= recommendedFees.fastestFee {
        let claimRequest = ClaimDepositRequest(
            txid: deposit.txid,
            vout: deposit.vout,
            maxFee: MaxFee.rate(satPerVbyte: requiredFeeRateSatPerVbyte)
        )
        try await sdk.claimDeposit(request: claimRequest)
    }
}
```

### Kotlin

```kotlin
try {
    val claimError = deposit.claimError
    if (claimError is DepositClaimError.MaxDepositClaimFeeExceeded) {
        val requiredFeeRate = claimError.requiredFeeRateSatPerVbyte

        val recommendedFees = sdk.recommendedFees()

        if (requiredFeeRate <= recommendedFees.fastestFee) {
            val claimRequest = ClaimDepositRequest(
                txid = deposit.txid,
                vout = deposit.vout,
                maxFee = MaxFee.Rate(requiredFeeRate)
            )
            sdk.claimDeposit(claimRequest)
        }
    }
} catch (e: Exception) {
    // handle error
}
```

### C#

```csharp
if (deposit.claimError is DepositClaimError.MaxDepositClaimFeeExceeded exceeded)
{
    var requiredFeeRate = exceeded.requiredFeeRateSatPerVbyte;

    var recommendedFees = await sdk.RecommendedFees();

    if (requiredFeeRate <= recommendedFees.fastestFee)
    {
        var claimRequest = new ClaimDepositRequest(
            txid: deposit.txid,
            vout: deposit.vout,
            maxFee: new MaxFee.Rate(satPerVbyte: requiredFeeRate)
        );
        await sdk.ClaimDeposit(request: claimRequest);
    }
}
```

### Javascript (Wasm)

```typescript
if (deposit.claimError?.type === 'maxDepositClaimFeeExceeded') {
  const requiredFeeRate = deposit.claimError.requiredFeeRateSatPerVbyte

  const recommendedFees = await sdk.recommendedFees()

  if (requiredFeeRate <= recommendedFees.fastestFee) {
    const claimRequest: ClaimDepositRequest = {
      txid: deposit.txid,
      vout: deposit.vout,
      maxFee: { type: 'rate', satPerVbyte: requiredFeeRate }
    }
    await sdk.claimDeposit(claimRequest)
  }
}
```

### React Native

```typescript
if (deposit.claimError?.tag === DepositClaimError_Tags.MaxDepositClaimFeeExceeded) {
  const requiredFeeRate = deposit.claimError.inner.requiredFeeRateSatPerVbyte

  const recommendedFees = await sdk.recommendedFees()

  if (requiredFeeRate <= recommendedFees.fastestFee) {
    const claimRequest: ClaimDepositRequest = {
      txid: deposit.txid,
      vout: deposit.vout,
      maxFee: new MaxFee.Rate({ satPerVbyte: requiredFeeRate })
    }
    await sdk.claimDeposit(claimRequest)
  }
}
```

### Flutter

```dart
final claimError = deposit.claimError;
if (claimError is DepositClaimError_MaxDepositClaimFeeExceeded) {
  final requiredFeeRate = claimError.requiredFeeRateSatPerVbyte;

  final recommendedFees = await sdk.recommendedFees();

  if (requiredFeeRate <= recommendedFees.fastestFee) {
    final claimRequest = ClaimDepositRequest(
      txid: deposit.txid,
      vout: deposit.vout,
      maxFee: MaxFee.rate(satPerVbyte: requiredFeeRate),
    );
    await sdk.claimDeposit(request: claimRequest);
  }
}
```

### Python

```python
try:
    if isinstance(
        deposit.claim_error, DepositClaimError.MAX_DEPOSIT_CLAIM_FEE_EXCEEDED
    ):
        required_fee_rate = deposit.claim_error.required_fee_rate_sat_per_vbyte

        recommended_fees = await sdk.recommended_fees()

        if required_fee_rate <= recommended_fees.fastest_fee:
            claim_request = ClaimDepositRequest(
                txid=deposit.txid,
                vout=deposit.vout,
                max_fee=MaxFee.RATE(sat_per_vbyte=required_fee_rate),
            )
            await sdk.claim_deposit(request=claim_request)
except Exception as error:
    logging.error(error)
    raise
```

### Go

```go
if claimErr := *deposit.ClaimError; claimErr != nil {
	if exceeded, ok := claimErr.(breez_sdk_spark.DepositClaimErrorMaxDepositClaimFeeExceeded); ok {
		requiredFeeRate := exceeded.RequiredFeeRateSatPerVbyte

		recommendedFees, err := sdk.RecommendedFees()
		if err != nil {
			return err
		}

		if requiredFeeRate <= recommendedFees.FastestFee {
			maxFee := breez_sdk_spark.MaxFee(breez_sdk_spark.MaxFeeRate{SatPerVbyte: requiredFeeRate})
			claimRequest := breez_sdk_spark.ClaimDepositRequest{
				Txid:   deposit.Txid,
				Vout:   deposit.Vout,
				MaxFee: &maxFee,
			}
			_, err := sdk.ClaimDeposit(claimRequest)
			if err != nil {
				var sdkErr *breez_sdk_spark.SdkError
				if errors.As(err, &sdkErr) {
					// Handle SdkError - can inspect specific variants if needed
					// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
				}
				return err
			}
		}
	}
}
```



## Recommended fees

Get Bitcoin fee estimates for different confirmation targets to help determine appropriate fee levels for claiming or refunding deposits.

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
