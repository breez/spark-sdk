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



Both limits must be given: to leave one open, set it to `TokenAllowanceLimit.UNLIMITED`.

Only one allowance can be active for the same spender and token. Granting another returns `TokenAllowanceErrorReason.ALREADY_ACTIVE`: list your allowances to find the active one. To change the limits, revoke the allowance and grant a new one.

## Listing allowances

Use `TokenAllowanceRole.OWNER` for allowances your wallet granted and `TokenAllowanceRole.SPENDER` for allowances granted to it.

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



The spent amount counts a pull as soon as it is sent, including pulls that later fail, so it can be higher than what was actually pulled.

## Revoking an allowance

```python
try:
    await sdk.revoke_token_allowance(
        request=RevokeTokenAllowanceRequest(allowance_id="<allowance id>")
    )
except Exception as error:
    logging.error(error)
    raise
```



A revocation takes effect within seconds. A pull that hasn't completed can still fail after a revocation, so treat a pull as paid only once it completes.

## Pulling a payment

A spender pulls in two steps. `prepare_pull_payment` finds the allowance, checks its limits and builds the transfer from the payer's tokens. `pull_payment` signs it and sends it.

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



Leave a receiver's address unset to pull into your own wallet. One pull can pay several receivers. When the allowance has a recipient list, every receiver must be on it, your own wallet included. The per-payment limit applies to the total.

### Retrying

A pull counts as paid once `pull_payment` returns it with the `status` `PaymentStatus.COMPLETED`. A `PaymentStatus.PENDING` pull is accepted but can still fail: call `pull_payment` again with the same prepare response to refresh it.

If `pull_payment` fails with `SdkError.SPARK_ERROR` or `SdkError.NETWORK_ERROR`, call it again with the same prepare response; the operators recognize a pull they've already seen, so this never charges the payer twice. Keep calling until the pull completes, fails with `SdkError.TOKEN_ALLOWANCE`, or 10 minutes have passed since its `expiry_time`, because a pull the operators accepted can still complete for a few minutes after that time. Prepare a new pull only after that.

`TokenAllowanceErrorReason.PREPARED_PULL_STALE` means the operators never accepted the pull and never will: the tokens it was prepared with were spent by another payment, or it wasn't sent before its window closed. Prepare a new pull right away. Don't call `pull_payment` twice at the same time with the same prepare response, because the second call can report a pull the first one is still sending as stale.

## Errors

Token allowance errors return `SdkError.TOKEN_ALLOWANCE` with a `reason` that says why the request was refused, for example `TokenAllowanceErrorReason.OVER_PER_PAYMENT_LIMIT`, `TokenAllowanceErrorReason.OVER_TOTAL_LIMIT`, `TokenAllowanceErrorReason.RECIPIENT_NOT_ALLOWED`, `TokenAllowanceErrorReason.REVOKED` or `TokenAllowanceErrorReason.PAYER_INSUFFICIENT_FUNDS`. `TokenAllowanceErrorReason.PREPARED_PULL_STALE` means the pull can't go through anymore (see [Retrying](#retrying)). `TokenAllowanceErrorReason.NOT_ENABLED` means token allowances aren't available on this network.

In JavaScript, errors arrive as text: the reason's name is part of the error message.

## Signing pulls on the client

To have the spender's key approve each pull on a separate device or signing service, see [Client signing](client_signing.md#pulls).
