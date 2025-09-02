import logging
from breez_sdk_spark import (
    BreezSdk,
    connect,
    ConnectRequest,
    default_config,
    default_storage,
    EventListener,
    GetInfoRequest,
    init_logging,
    LogEntry,
    Logger,
    Network,
    SdkBuilder,
    SdkEvent,
)


async def init_sdk():
    # ANCHOR: init-sdk
    mnemonic = "<mnemonic words>"
    # Create the default config
    config = default_config(network=Network.MAINNET)
    config.api_key = "<breez api key>"

    try:
        # Connect to the SDK using the simplified connect method
        sdk = await connect(
            request=ConnectRequest(
                config=config, mnemonic=mnemonic, storage_dir="./.data"
            )
        )
        return sdk
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: init-sdk


async def init_sdk_advanced():
    # ANCHOR: init-sdk-advanced
    mnemonic = "<mnemonic words>"
    # Create the default config
    config = default_config(network=Network.MAINNET)
    config.api_key = "<breez api key>"

    try:
        # Create the default storage
        storage = default_storage(data_dir="./.data")

        # Build the SDK using the config, mnemonic and storage
        builder = SdkBuilder(config=config, mnemonic=mnemonic, storage=storage)

        # You can also pass your custom implementations:
        # builder.with_chain_service(<your chain service implementation>)
        # builder.with_rest_client(<your rest client implementation>)
        sdk = await builder.build()
        return sdk
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: init-sdk-advanced


async def fetch_balance(sdk: BreezSdk):
    # ANCHOR: fetch-balance
    try:
        info = await sdk.get_info(request=GetInfoRequest())
        balance_sats = info.balance_sats
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: fetch-balance


# ANCHOR: logging
class SdkLogger(Logger):
    def log(self, l: LogEntry):
        logging.debug(f"Received log [{l.level}]: {l.line}")


def set_logger(logger: SdkLogger):
    try:
        init_logging(log_dir=None, app_logger=logger, log_filter=None)
    except Exception as error:
        logging.error(error)
        raise


# ANCHOR_END: logging


# ANCHOR: add-event-listener
class SdkListener(EventListener):
    def on_event(self, event: SdkEvent):
        logging.debug(f"Received event {event}")


def add_event_listener(sdk: BreezSdk, listener: SdkListener):
    try:
        listener_id = sdk.add_event_listener(listener=listener)
        return listener_id
    except Exception as error:
        logging.error(error)
        raise


# ANCHOR_END: add-event-listener


# ANCHOR: remove-event-listener
def remove_event_listener(sdk: BreezSdk, listener_id: str):
    try:
        sdk.remove_event_listener(listener_id=listener_id)
    except Exception as error:
        logging.error(error)
        raise


# ANCHOR_END: remove-event-listener


# ANCHOR: disconnect
def disconnect(sdk: BreezSdk):
    try:
        sdk.disconnect()
    except Exception as error:
        logging.error(error)
        raise


# ANCHOR_END: disconnect
