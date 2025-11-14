# Handling unclaimed deposits

When receiving Bitcoin payments through onchain deposits, the SDK automatically attempts to claim these funds to make them available in your wallet. However, there are scenarios where the deposit claiming process may fail, requiring manual intervention to either retry the claim or refund the deposit to an external Bitcoin address.

## Understanding why deposits are unclaimed

Unclaimed deposits can happen for several reasons:

- **Insufficient fee configuration**: The maximum configured fees may be not set or too low to process the claim transaction during periods of high network congestion. See [Recommended fees](#recommended-fees) to check the current recommended fees.
- **UTXO unavailability**: The deposit UTXO may no longer be available or has been spent elsewhere
- **Other unexpected errors**: Various technical issues that prevent successful claiming

The SDK emits a `UnclaimedDeposits` event containing information about the unclaimed deposits, including the specific reason why the deposit is unclaimed.

## Managing unclaimed deposits

The SDK provides three methods to handle unclaimed deposits:

1. **Listing unclaimed deposits** - Retrieve all deposits that have not yet been claimed
2. **Claiming a deposit** - Claim a deposit using specific claiming parameters
3. **Refunding a deposit** - Send the deposit funds to an external Bitcoin address

### Listing unclaimed deposits

This lists all of the currently unclaimed deposits, including the specific reason why the deposit is unclaimed.

{{#tabs refunding_payments:list-unclaimed-deposits}}

### Claiming a deposit

If a deposit is unclaimed due to insufficient fees, you can retry the claim operation with a higher maximum fee. This is particularly useful during periods of high network congestion when transaction fees are elevated. See [Recommended fees](#recommended-fees) to check the current recommended fees.

{{#tabs refunding_payments:claim-deposit}}

### Refunding a deposit

When a deposit cannot be successfully claimed, you can refund the funds to an external Bitcoin address. This operation creates a transaction that sends the deposit amount (minus transaction fees) to the specified destination address.

{{#tabs refunding_payments:refund-deposit}}

## Best Practices

- **Monitor events**: Listen for `UnclaimedDeposits` events to be notified when deposits require manual intervention
- **Check claim errors**: Examine the `claim_error` field in deposit information to understand why claims failed
- **Fee management**: For fee-related failures, consider retrying with higher maximum fees during network congestion
- **Refund**: Use refunding when claims consistently fail or when you need immediate access to funds and want to avoid the double-fee scenario (claim fee + cooperative exit fee)

## Recommended fees

You can get the Bitcoin fee estimates for different confirmation targets.

{{#tabs refunding_payments:recommended-fees}}
