import logging
import typing
from breez_sdk_spark import (
    default_config,
    default_postgres_storage_config,
    Network,
    ProvisionalPayment,
    SdkBuilder,
    Seed,
    PaymentObserver,
    ChainApiType,
    Credentials,
    KeySetType,
    KeySetConfig,
)


async def init_sdk_advanced():
    # ANCHOR: init-sdk-advanced
    # Construct the seed using mnemonic words or entropy bytes
    mnemonic = "<mnemonic words>"
    seed = Seed.MNEMONIC(mnemonic=mnemonic, passphrase=None)
    # Create the default config
    config = default_config(network=Network.MAINNET)
    config.api_key = "<breez api key>"
    try:
        # Build the SDK using the config, seed and default storage
        builder = SdkBuilder(config=config, seed=seed)
        await builder.with_default_storage(storage_dir="./.data")
        # You can also pass your custom implementations:
        # await builder.with_storage(<your storage implementation>)
        # await builder.with_chain_service(<your chain service implementation>)
        # await builder.with_rest_client(<your rest client implementation>)
        # await builder.with_key_set(<your key set type>, <use address index>, <account number>)
        # await builder.with_payment_observer(<your payment observer implementation>)
        sdk = await builder.build()
        return sdk
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: init-sdk-advanced


async def with_rest_chain_service(builder: SdkBuilder):
    # ANCHOR: with-rest-chain-service
    url = "<your REST chain service URL>"
    chain_api_type = ChainApiType.MEMPOOL_SPACE
    optional_credentials = Credentials(
        username="<username>",
        password="<password>",
    )
    await builder.with_rest_chain_service(
        url=url,
        api_type=chain_api_type,
        credentials=optional_credentials,
    )
    # ANCHOR_END: with-rest-chain-service


async def with_key_set(builder: SdkBuilder):
    # ANCHOR: with-key-set
    key_set_type = KeySetType.DEFAULT
    use_address_index = False
    optional_account_number = 21

    key_set_config = KeySetConfig(
        key_set_type=key_set_type,
        use_address_index=use_address_index,
        account_number=optional_account_number,
    )

    await builder.with_key_set(config=key_set_config)
    # ANCHOR_END: with-key-set


# ANCHOR: with-payment-observer
class ExamplePaymentObserver(PaymentObserver):
    def before_send(self, payments: typing.List[ProvisionalPayment]):
        for payment in payments:
            logging.debug(f"About to send payment {payment.payment_id} of amount {payment.amount}")


async def with_payment_observer(builder: SdkBuilder):
    payment_observer = ExamplePaymentObserver()
    await builder.with_payment_observer(payment_observer=payment_observer)
# ANCHOR_END: with-payment-observer


# ANCHOR: init-sdk-postgres
async def init_sdk_postgres():
    # Construct the seed using mnemonic words or entropy bytes
    mnemonic = "<mnemonic words>"
    seed = Seed.MNEMONIC(mnemonic=mnemonic, passphrase=None)

    # Create the default config
    config = default_config(network=Network.MAINNET)
    config.api_key = "<breez api key>"

    # Configure PostgreSQL storage
    # Connection string format: "host=localhost user=postgres password=secret dbname=spark"
    # Or URI format: "postgres://user:password@host:port/dbname?sslmode=require"
    postgres_config = default_postgres_storage_config(
        connection_string="host=localhost user=postgres dbname=spark"
    )
    # Optionally pool settings can be adjusted. Some examples:
    postgres_config.max_pool_size = 8  # Max connections in pool
    postgres_config.wait_timeout_secs = 30  # Timeout waiting for connection

    try:
        # Build the SDK with PostgreSQL storage
        builder = SdkBuilder(config=config, seed=seed)
        await builder.with_postgres_storage(config=postgres_config)
        sdk = await builder.build()
        return sdk
    except Exception as error:
        logging.error(error)
        raise
# ANCHOR_END: init-sdk-postgres
