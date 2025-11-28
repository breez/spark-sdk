# Claiming on-chain deposits

When receiving bitcoin through on-chain deposits, the SDK [automatically attempts](#setting-a-max-fee-for-automatic-claims) to claim these funds to make them available in your balance. However, if the maximum deposit claim fee is too low for claiming deposits, they won't be automatically claimed and should be [manually claimed](#manually-claiming-deposits).

## Setting a max fee for automatic claims

The [maximum deposit claim fee](config.md#max-deposit-claim-fee) setting in the SDK configuration defines the maximum fee the SDK uses when automatically claiming an on-chain deposit. The SDK's default fee limit is set to 1 sats/vbyte, which is low and requires manual claiming when fees exceed this threshold. You can set a higher fee, either in sats/vbyte or in absolute sats, to automatically claim deposits.

One possible approach is to set the maximum claim fee according to the current market conditions using the [recommended fees](#recommended-fees) API.

{{#tabs refunding_payments:set-max-fee-to-recommended-fees}}

<div class="warning">
<h4>Developer note</h4>

Even when setting a high fee, the SDK might still fail to automatically claim deposits. In these cases, it's recommended to manually claim them by letting the end user accept the required fees. When [manual intervention](#manually-claiming-deposits) is required, the SDK emits an `UnclaimedDeposits` event containing information about the deposit. See [Listening to events](events.md) for how to subscribe to events.

</div>

## Manually claiming deposits

When a deposit cannot be automatically claimed due to the configured maximum fee being too low, you can manually claim it by specifying a higher fee limit. The recommended approach is to display a user interface showing the required fee amount and request user approval before proceeding with manual claiming.

{{#tabs refunding_payments:handle-fee-exceeded}}

## Listing unclaimed deposits

Retrieve all deposits that have not yet been claimed, including the specific reason why each deposit is unclaimed.

{{#tabs refunding_payments:list-unclaimed-deposits}}

## Refunding deposits

When a deposit cannot be successfully claimed you can refund it to an external Bitcoin address. This creates a transaction that sends the amount (minus transaction fees) to the specified destination address.

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for refund transactions.

{{#tabs refunding_payments:refund-deposit}}

## Implementing a custom claim logic

For advanced use cases, you may want to implement a custom claim logic instead of relying on the SDK's automatic process. This gives you complete control over when and how deposits are claimed.

To disable automatic claims, unset the [maximum deposit claim fee](config.md#max-deposit-claim-fee). Then use the methods described above to manually claim deposits based on your business logic.

Common scenarios for custom claiming logic include:

- **Dynamic fee adjustment**: Adjust claiming fees based on market conditions or priority
- **Conditional claiming**: Only claim deposits that meet certain criteria (amount thresholds, time windows, etc.)
- **Integration with external systems**: Coordinate claims with other business processes

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for claiming deposits.

## Recommended fees

Get Bitcoin fee estimates for different confirmation targets to help determine appropriate fee levels for claiming or refunding deposits.

{{#tabs refunding_payments:recommended-fees}}
