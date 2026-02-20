"""Top-level namespace for the Breez SDK Spark.

Groups all static/global SDK functions that don't require a wallet
connection. Use ``BreezSdkSpark.connect()`` to obtain a ``BreezSparkClient`` instance.
"""

from breez_sdk_spark.breez_sdk_spark import (
    BreezSdk as BreezSparkClient,
    connect as _connect,
    connect_with_signer as _connect_with_signer,
    default_config as _default_config,
    default_external_signer as _default_external_signer,
    get_spark_status as _get_spark_status,
    init_logging as _init_logging,
    parse as _parse,
)


class BreezSdkSpark:
    """Top-level namespace for the Breez SDK Spark."""

    @staticmethod
    def default_config(network):
        """Returns a default SDK configuration for the given network."""
        return _default_config(network)

    @staticmethod
    def init_logging(log_dir=None, app_logger=None, log_filter=None):
        """Initializes the SDK logging subsystem."""
        return _init_logging(log_dir, app_logger, log_filter)

    @staticmethod
    async def connect(request):
        """Connects to the Spark network using the provided configuration and seed."""
        return await _connect(request)

    @staticmethod
    async def connect_with_signer(request):
        """Connects to the Spark network using an external signer."""
        return await _connect_with_signer(request)

    @staticmethod
    def default_external_signer(mnemonic, passphrase=None, network=None, key_set_config=None):
        """Creates a default external signer from a mnemonic phrase."""
        return _default_external_signer(mnemonic, passphrase, network, key_set_config)

    @staticmethod
    async def parse(input, external_input_parsers=None):
        """Parses a payment input string and returns the identified type."""
        return await _parse(input, external_input_parsers)

    @staticmethod
    async def get_spark_status():
        """Fetches the current status of Spark network services."""
        return await _get_spark_status()
