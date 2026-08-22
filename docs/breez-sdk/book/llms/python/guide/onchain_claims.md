# Claiming on-chain deposits

On-chain deposits go through three stages: once detected, the deposit is visible in the SDK and each deposit includes a `is_mature` field; after **3 on-chain confirmations** the deposit has sufficient confirmations (`is_mature` is true) and the SDK [automatically attempts](#setting-a-max-fee-for-automatic-claims) to claim it. If the maximum deposit claim fee is too low, the deposit won't be automatically claimed and should be [manually claimed](#manually-claiming-deposits).

## Setting a max fee for automatic claims

The [maximum deposit claim fee](config.md#max-deposit-claim-fee) setting in the SDK configuration defines the maximum fee the SDK uses when automatically claiming an on-chain deposit. The SDK's default fee limit is set to 1 sats/vbyte, which is low and requires manual claiming when fees exceed this threshold. You can set a higher fee, either in sats/vbyte, in absolute sats, or to the fastest recommended fee at the time of claim, with a leeway in sats/vbyte.

To increase the likelihood of automatically claiming deposits, you may set the maximum fee to the fastest recommended rate at the time of claim, which can result in higher fees.

```python
# Create the default config
config = default_config(network=Network.MAINNET)
config.api_key = "<breez api key>"

# Set the maximum fee to the fastest network recommended fee at the time of claim
# with a leeway of 1 sats/vbyte
config.max_deposit_claim_fee = MaxFee.NETWORK_RECOMMENDED(leeway_sat_per_vbyte=1)
```



However, even when setting a high fee, the SDK might still fail to automatically claim deposits. In these cases, it's recommended to manually claim them by letting the end user accept the required fees. When [manual intervention](#manually-claiming-deposits) is required, the SDK emits an `SdkEvent.UNCLAIMED_DEPOSITS` event containing information about the deposit. See [Listening to events](events.md) for how to subscribe to events.

## Manually claiming deposits

When a deposit cannot be automatically claimed due to the configured maximum fee being too low, you can manually claim it by specifying a higher fee limit. The recommended approach is to display a user interface showing the required fee amount and request user approval before proceeding with manual claiming.

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



## Listing unclaimed deposits

Retrieve all deposits that have not yet been claimed. This includes pending deposits that do not yet have sufficient confirmations, as well as deposits with sufficient confirmations that failed to claim (with the specific failure reason). Pending deposits will be automatically claimed once they have sufficient confirmations.

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



## Refunding deposits

When a deposit cannot be successfully claimed you can refund it to an external Bitcoin address. This creates a transaction that sends the amount (minus transaction fees) to the specified destination address.

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for refund transactions.

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



**Developer note**

The total fee must be at least 194 sats to ensure the transaction can be relayed by the Bitcoin network. If the fee is lower, the refund request will be rejected.

## Implementing a custom claim logic

For advanced use cases, you may want to implement a custom claim logic instead of relying on the SDK's automatic process. This gives you complete control over when and how deposits are claimed.

To disable automatic claims, unset the [maximum deposit claim fee](config.md#max-deposit-claim-fee). Then use the methods described above to manually claim deposits based on your business logic.

Common scenarios for custom claiming logic include:

- **Dynamic fee adjustment**: Adjust claiming fees based on market conditions or priority
- **Conditional claiming**: Only claim deposits that meet certain criteria (amount thresholds, time windows, etc.)
- **Integration with external systems**: Coordinate claims with other business processes

The [recommended fees](#recommended-fees) API is useful for determining appropriate fee levels for claiming deposits. For example, you can implement a custom claim logic to only claim deposits if the required fee rate is less than the fastest recommended fee (or any other).

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



## Recommended fees

Get Bitcoin fee estimates for different confirmation targets to help determine appropriate fee levels for claiming or refunding deposits.

```python
response = await sdk.recommended_fees()
logging.info(f"Fastest fee: {response.fastest_fee} sats/vByte")
logging.info(f"Half-hour fee: {response.half_hour_fee} sats/vByte")
logging.info(f"Hour fee: {response.hour_fee} sats/vByte")
logging.info(f"Economy fee: {response.economy_fee} sats/vByte")
logging.info(f"Minimum fee: {response.minimum_fee} sats/vByte")
```
