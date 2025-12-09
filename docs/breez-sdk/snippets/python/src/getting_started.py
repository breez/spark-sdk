import logging
from breez_sdk_spark import (
    BreezSdk,
    connect,
    ConnectRequest,
    default_config,
    EventListener,
    GetInfoRequest,
    init_logging,
    LogEntry,
    Logger,
    Network,
    SdkEvent,
    Seed,
)


async def init_sdk():
    # ANCHOR: init-sdk
    # Construct the seed using mnemonic words or entropy bytes
    mnemonic = "<mnemonic words>"
    seed = Seed.MNEMONIC(mnemonic=mnemonic, passphrase=None)
    # Create the default config
    config = default_config(network=Network.MAINNET)
    config.api_key = "<breez api key>"
    try:
        # Connect to the SDK using the simplified connect method
        sdk = await connect(
            request=ConnectRequest(config=config, seed=seed, storage_dir="./.data")
        )
        return sdk
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: init-sdk


async def fetch_balance(sdk: BreezSdk):
    # ANCHOR: fetch-balance
    try:
        # ensure_synced: True will ensure the SDK is synced with the Spark network
        # before returning the balance
        info = await sdk.get_info(request=GetInfoRequest(ensure_synced=False))
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
        if isinstance(event, SdkEvent.SYNCED):
            # Data has been synchronized with the network. When this event is received,
            # it is recommended to refresh the payment list and wallet balance.
            pass
        elif isinstance(event, SdkEvent.UNCLAIMED_DEPOSITS):
            # SDK was unable to claim some deposits automatically
            unclaimed_deposits = event.unclaimed_deposits
        elif isinstance(event, SdkEvent.CLAIMED_DEPOSITS):
            # Deposits were successfully claimed
            claimed_deposits = event.claimed_deposits
        elif isinstance(event, SdkEvent.PAYMENT_SUCCEEDED):
            # A payment completed successfully
            payment = event.payment
        elif isinstance(event, SdkEvent.PAYMENT_PENDING):
            # A payment is pending (waiting for confirmation)
            pending_payment = event.payment
        elif isinstance(event, SdkEvent.PAYMENT_FAILED):
            # A payment failed
            failed_payment = event.payment
        else:
            # Handle any future event types
            pass


async def add_event_listener(sdk: BreezSdk, listener: SdkListener):
    try:
        listener_id = await sdk.add_event_listener(listener=listener)
        return listener_id
    except Exception as error:
        logging.error(error)
        raise


# ANCHOR_END: add-event-listener


# ANCHOR: remove-event-listener
async def remove_event_listener(sdk: BreezSdk, listener_id: str):
    try:
        await sdk.remove_event_listener(id=listener_id)
    except Exception as error:
        logging.error(error)
        raise


# ANCHOR_END: remove-event-listener


# ANCHOR: disconnect
async def disconnect(sdk: BreezSdk):
    try:
        await sdk.disconnect()
    except Exception as error:
        logging.error(error)
        raise


# ANCHOR_END: disconnect
