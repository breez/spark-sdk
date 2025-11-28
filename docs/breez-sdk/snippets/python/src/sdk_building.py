import logging
import typing
from breez_sdk_spark import (
    default_config,
    Network,
    ProvisionalPayment,
    SdkBuilder,
    Seed,
    PaymentObserver,
    Credentials,
    KeySetType,
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
        # await builder.with_real_time_sync_storage(<your real-time sync storage implementation>)
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
    optional_credentials = Credentials(
        username="<username>",
        password="<password>",
    )
    await builder.with_rest_chain_service(
        url=url,
        credentials=optional_credentials,
    )
    # ANCHOR_END: with-rest-chain-service


async def with_key_set(builder: SdkBuilder):
    # ANCHOR: with-key-set
    key_set_type = KeySetType.DEFAULT
    use_address_index = False
    optional_account_number = 21
    await builder.with_key_set(
        key_set_type=key_set_type,
        use_address_index=use_address_index,
        account_number=optional_account_number,
    )
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
