# Sending payments using LNURL-Pay and Lightning address

## Preparing LNURL Payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_lnurl_pay

During the prepare step, the SDK ensures that the inputs are valid with respect to the LNURL-pay request,
and also returns the fees related to the payment so they can be confirmed.

Payments can be sent without holding Bitcoin by converting on-the-fly as a step before sending a payment. See <a href="./token_conversion.md">Converting tokens</a> for more information.

### Setting the receiver amount

When you want the payment recipient to receive a specific amount.

```python
# Endpoint can also be of the form:
# lnurlp://domain.com/lnurl-pay?key=val
# lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43r
#     vv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3k
#     vdnxx5crxwpjvyunsephsz36jf
lnurl_pay_url = "lightning@address.com"
try:
    parsed_input = await sdk.parse(lnurl_pay_url)
    if isinstance(parsed_input, InputType.LIGHTNING_ADDRESS):
        details = parsed_input[0]
        amount_sats = 5_000
        optional_comment = "<comment>"
        pay_request = details.pay_request
        optional_validate_success_action_url = True

        request = PrepareLnurlPayRequest(
            amount=amount_sats,
            pay_request=pay_request,
            comment=optional_comment,
            validate_success_action_url=optional_validate_success_action_url,
            token_identifier=None,
            conversion_options=None,
            fee_policy=None,
        )
        prepare_response = await sdk.prepare_lnurl_pay(request=request)

        # If the fees are acceptable, continue to create the LNURL Pay
        logging.debug(f"Fees: {prepare_response.fee_sats} sats")
        return prepare_response
except Exception as error:
    logging.error(error)
    raise
```



### Setting the fee policy

By default, fees are added on top of the amount (`FeePolicy.FEES_EXCLUDED`). Use `FeePolicy.FEES_INCLUDED` to deduct fees from the amount instead—the receiver gets the amount minus fees.

This is particularly useful when you want to spend your entire balance in a single payment—simply provide your full balance as the amount.

```python
# By default (FeePolicy.FEES_EXCLUDED), fees are added on top of the amount.
# Use FeePolicy.FEES_INCLUDED to deduct fees from the amount instead.
# The receiver gets amount minus fees.
amount_sats = 5_000
optional_comment = "<comment>"
optional_validate_success_action_url = True

request = PrepareLnurlPayRequest(
    amount=amount_sats,
    pay_request=pay_request,
    comment=optional_comment,
    validate_success_action_url=optional_validate_success_action_url,
    token_identifier=None,
    conversion_options=None,
    fee_policy=FeePolicy.FEES_INCLUDED,
)
prepare_response = await sdk.prepare_lnurl_pay(request=request)

# If the fees are acceptable, continue to create the LNURL Pay
fee_sats = prepare_response.fee_sats
logging.debug(f"Fees: {fee_sats} sats")
# The receiver gets amount_sats - fee_sats
```



### Sending entire token balance

When [stable balance](./stable_balance.md) is active, you can send your entire wallet balance via LNURL. See [Sending entire balance](./stable_balance.md#sending-entire-balance) for details.

## LNURL Payments

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.lnurl_pay

Once the payment has been prepared and the fees are accepted, the payment can be sent by passing:
- **Prepare Response** - The response from the [Preparing LNURL Payments](lnurl_pay.md#preparing-lnurl-payments) step.
- **Idempotency Key** - An optional UUID that identifies the payment. If set, providing the same idempotency key for multiple requests will ensure that only one payment is made.

```python
try:
    optional_idempotency_key = "<idempotency key uuid>"
    response = await sdk.lnurl_pay(
        LnurlPayRequest(
            prepare_response=prepare_response,
            idempotency_key=optional_idempotency_key,
        )
    )
except Exception as error:
    logging.error(error)
    raise
```



**Developer note**

By default when the LNURL-pay results in a success action with a URL, the URL is validated to check if there is a mismatch with the LNURL callback domain. You can disable this behaviour by setting the optional validation <code>PrepareLnurlPayRequest</code> param to false.

## Managing contacts

You can save frequently used Lightning addresses as contacts for quick access. See [Managing contacts](contacts.md) for details.

## Supported Specs

- [LUD-01](https://github.com/lnurl/luds/blob/luds/01.md) LNURL bech32 encoding
- [LUD-06](https://github.com/lnurl/luds/blob/luds/06.md) `payRequest` spec
- [LUD-09](https://github.com/lnurl/luds/blob/luds/09.md) `successAction` field for `payRequest`
- [LUD-16](https://github.com/lnurl/luds/blob/luds/16.md) LN Address
- [LUD-17](https://github.com/lnurl/luds/blob/luds/17.md) Support for lnurlp prefix with non-bech32-encoded LNURL URLs
