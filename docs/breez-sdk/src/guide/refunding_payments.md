# Refunding Payments

When receiving Bitcoin payments through onchain deposits, the SDK automatically attempts to claim these funds to make them available in your wallet. However, there are scenarios where the deposit claiming process may fail, requiring manual intervention to either retry the claim or refund the deposit to an external Bitcoin address.

## Understanding Deposit Claim Failures

Deposit claims can fail for several reasons:

- **Insufficient fee configuration**: The maximum configured fees may be too low to process the claim transaction during periods of high network congestion
- **UTXO unavailability**: The deposit UTXO may no longer be available or has been spent elsewhere
- **Other unexpected errors**: Various technical issues that prevent successful claiming

When deposit claims fail, the SDK emits a `ClaimDepositsFailed` event containing information about the unclaimed deposits, including the specific error that caused each failure.

## Managing Failed Deposits

The SDK provides three methods to handle failed deposit claims:

1. **List unclaimed deposits** - Retrieve all deposits that failed to be claimed
2. **Retry claiming** - Attempt to claim a deposit again with different parameters
3. **Refund deposit** - Send the deposit funds to an external Bitcoin address

## Listing Unclaimed Deposits

Use the `list_unclaimed_deposits` method to retrieve all deposits that have failed to be claimed. This method returns detailed information about each failed deposit, including the specific error that caused the failure.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/refunding_payments.rs:list-unclaimed-deposits}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/RefundingPayments.swift:list-unclaimed-deposits}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/RefundingPayments.kt:list-unclaimed-deposits}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/refunding_payments.ts:list-unclaimed-deposits}}
```
</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/refunding_payments.dart:list-unclaimed-deposits}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore 
{{#include ../../snippets/python/src/refunding_payments.py:list-unclaimed-deposits}}
```
</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/refunding_payments.go:list-unclaimed-deposits}}
```
</section>
</custom-tabs>

## Retrying Deposit Claims

If a deposit claim failed due to insufficient fees, you can retry the claim operation with a higher maximum fee. This is particularly useful during periods of high network congestion when transaction fees are elevated.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/refunding_payments.rs:claim-deposit}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/RefundingPayments.swift:claim-deposit}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/RefundingPayments.kt:claim-deposit}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/refunding_payments.ts:claim-deposit}}
```
</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/refunding_payments.dart:claim-deposit}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore 
{{#include ../../snippets/python/src/refunding_payments.py:claim-deposit}}
```
</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/refunding_payments.go:claim-deposit}}
```
</section>
</custom-tabs>

## Refunding Deposits

When a deposit cannot be successfully claimed, you can refund the funds to an external Bitcoin address. This operation creates a transaction that sends the deposit amount (minus transaction fees) to the specified destination address.

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,ignore
{{#include ../../snippets/rust/src/refunding_payments.rs:refund-deposit}}
```
</section>

<div slot="title">Swift</div>
<section>

```swift,ignore
{{#include ../../snippets/swift/BreezSdkSnippets/Sources/RefundingPayments.swift:refund-deposit}}
```
</section>

<div slot="title">Kotlin</div>
<section>

```kotlin,ignore
{{#include ../../snippets/kotlin_mpp_lib/shared/src/commonMain/kotlin/com/example/kotlinmpplib/RefundingPayments.kt:refund-deposit}}
```
</section>

<div slot="title">Javascript</div>
<section>

```typescript
{{#include ../../snippets/wasm/refunding_payments.ts:refund-deposit}}
```
</section>

<div slot="title">Flutter</div>
<section>

```dart,ignore
{{#include ../../snippets/flutter/lib/refunding_payments.dart:refund-deposit}}
```
</section>

<div slot="title">Python</div>
<section>

```python,ignore 
{{#include ../../snippets/python/src/refunding_payments.py:refund-deposit}}
```
</section>

<div slot="title">Go</div>
<section>

```go,ignore
{{#include ../../snippets/go/refunding_payments.go:refund-deposit}}
```
</section>
</custom-tabs>

## Best Practices

- **Monitor events**: Listen for `ClaimDepositsFailed` events to be notified when deposits require manual intervention
- **Check claim errors**: Examine the `claim_error` field in deposit information to understand why claims failed
- **Fee management**: For fee-related failures, consider retrying with higher maximum fees during network congestion
- **Refund**: Use refunding when claims consistently fail or when you need immediate access to funds and want to avoid the double-fee scenario (claim fee + cooperative exit fee)