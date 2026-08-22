## Issuing tokens

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.get_token_issuer

The Breez SDK provides a specialized Token Issuer interface for managing custom token issuance on the Spark network using the using the [BTKN protocol](https://docs.spark.money/learn/tokens/hello-btkn). This functionality enables token creators to issue, manage, and control their own tokens with advanced features.

```python
token_issuer = sdk.get_token_issuer()
```



## Token creation

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.create_issuer_token

Create a custom token with configurable parameters. Define the decimal precision, max supply and if the token can be frozen.

```python
try:
    request = CreateIssuerTokenRequest(
        name="My Token",
        ticker="MTK",
        decimals=6,
        is_freezable=False,
        max_supply=1_000_000,
    )
    token_metadata = await token_issuer.create_issuer_token(request)
    logging.debug(f"Token identifier: {token_metadata.identifier}")
except Exception as error:
    logging.error(error)
    raise
```



### Creating multiple tokens

Token creation is limited to one token per issuer wallet. If you need to create and then manage more than one token using the same mnemonic, we recommend using different account numbers when initializing the SDK.

```python
account_number = 21

mnemonic = "<mnemonic words>"
seed = Seed.MNEMONIC(mnemonic=mnemonic, passphrase=None)
config = default_config(network=Network.MAINNET)
config.api_key = "<breez api key>"
try:
    builder = SdkBuilder(config=config, seed=seed)
    await builder.with_default_storage(storage_dir="./.data")

    # Set the account number for the SDK
    await builder.with_account_number(account_number=account_number)

    sdk = await builder.build()
    return sdk
except Exception as error:
    logging.error(error)
    raise
```



## Supply Management

### Minting a token

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.mint_issuer_token

Mint to increase the circulating supply of the token.

```python
try:
    request = MintIssuerTokenRequest(
        amount=1_000,
    )
    payment = await token_issuer.mint_issuer_token(request)
except Exception as error:
    logging.error(error)
    raise
```



### Burning a token

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.TokenIssuer.html#method.burn_issuer_token

Permanently remove tokens from the circulating supply by burning them.

```python
try:
    request = BurnIssuerTokenRequest(
        amount=1_000,
    )
    payment = await token_issuer.burn_issuer_token(request)
except Exception as error:
    logging.error(error)
    raise
```



### Listing mint or burn payments

Mint or burn payments are included in the regular payment history that is obtained when [Listing payments](./list_payments.md).

You can filter by token transaction type to only include mint, burn or transfer payments. Transfer payments are regular token payments that are not mint or burn payments.

```python
# Provide one or multiple of the following filters to
# the `payment_details_filter` field when listing payments
payment_details_transfer_filter = PaymentDetailsFilter.TOKEN(
    tx_type=TokenTransactionType.TRANSFER,
    tx_hash=None,
    conversion_refund_needed=None
)
payment_details_mint_filter = PaymentDetailsFilter.TOKEN(
    tx_type=TokenTransactionType.MINT,
    tx_hash=None,
    conversion_refund_needed=None
)
payment_details_burn_filter = PaymentDetailsFilter.TOKEN(
    tx_type=TokenTransactionType.BURN,
    tx_hash=None,
    conversion_refund_needed=None
)
```



## Query balance & metadata

Retrieve the current issued token balance and fetch the token metadata.

```python
try:
    token_balance = await token_issuer.get_issuer_token_balance()
    logging.debug(f"Token balance: {token_balance.balance}")

    token_metadata = await token_issuer.get_issuer_token_metadata()
    logging.debug(f"Token ticker: {token_metadata.ticker}")
except Exception as error:
    logging.error(error)
    raise
```



## Freeze and unfreeze tokens

Freeze and unfreeze tokens at a specific Spark address if the token metadata allows it.

```python
try:
    spark_address = "<spark address>"
    # Freeze the tokens held at the specified Spark address
    freeze_request = FreezeIssuerTokenRequest(
        address=spark_address,
    )
    freeze_response = await token_issuer.freeze_issuer_token(freeze_request)
    # Unfreeze the tokens held at the specified Spark address
    unfreeze_request = UnfreezeIssuerTokenRequest(
        address=spark_address,
    )
    unfreeze_response = await token_issuer.unfreeze_issuer_token(unfreeze_request)
except Exception as error:
    logging.error(error)
    raise
```
