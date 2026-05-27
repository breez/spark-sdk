import logging
import typing
from breez_sdk_spark import (
    BreezSdk,
    default_config,
    default_server_config,
    default_postgres_storage_config,
    create_postgres_connection_pool,
    default_mysql_storage_config,
    create_mysql_connection_pool,
    Network,
    ProvisionalPayment,
    ReceivePaymentMethod,
    ReceivePaymentRequest,
    SdkBuilder,
    Seed,
    PaymentObserver,
    ChainApiType,
    Credentials,
    KeySetType,
    KeySetConfig,
    UpdateUserSettingsRequest,
)


async def init_sdk_advanced():
    # ANCHOR: init-sdk-advanced
    # Construct the seed using a mnemonic, entropy or passkey
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
    async def before_send(self, payments: typing.List[ProvisionalPayment]):
        for payment in payments:
            logging.debug(f"About to send payment {payment.payment_id} of amount {payment.amount}")


async def with_payment_observer(builder: SdkBuilder):
    payment_observer = ExamplePaymentObserver()
    await builder.with_payment_observer(payment_observer=payment_observer)
# ANCHOR_END: with-payment-observer


# ANCHOR: init-sdk-postgres
async def init_sdk_postgres():
    # Construct the seed using a mnemonic, entropy or passkey
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
    # If your service owns SDK-compatible schema migrations:
    postgres_config.run_migration = False

    # Construct the connection pool. The same pool can be passed to multiple
    # SdkBuilders to share connections across SDKs; per-tenant scoping (rows
    # isolated by seed identity) is preserved.
    pool = create_postgres_connection_pool(config=postgres_config)

    try:
        # Build the SDK with PostgreSQL backend (storage, tree store, and token store)
        builder = SdkBuilder(config=config, seed=seed)
        await builder.with_postgres_connection_pool(pool=pool)
        sdk = await builder.build()
        return sdk
    except Exception as error:
        logging.error(error)
        raise
# ANCHOR_END: init-sdk-postgres


# ANCHOR: init-sdk-mysql
async def init_sdk_mysql():
    # Construct the seed using a mnemonic, entropy or passkey
    mnemonic = "<mnemonic words>"
    seed = Seed.MNEMONIC(mnemonic=mnemonic, passphrase=None)

    # Create the default config
    config = default_config(network=Network.MAINNET)
    config.api_key = "<breez api key>"

    # Configure MySQL backend (MySQL 8.0+).
    # Connection string format (URL only):
    #   "mysql://user:password@host:3306/dbname?ssl-mode=required"
    mysql_config = default_mysql_storage_config(
        connection_string="mysql://user:password@localhost:3306/spark"
    )
    # Optionally pool settings can be adjusted. Some examples:
    mysql_config.max_pool_size = 8  # Max connections in pool
    mysql_config.recycle_timeout_secs = 60  # Recycle idle connections after this many seconds
    # Provide a custom CA certificate when using ssl-mode=verify_ca or verify_identity:
    # mysql_config.root_ca_pem = "-----BEGIN CERTIFICATE-----\n..."

    # Construct the connection pool. The same pool can be passed to multiple
    # SdkBuilders to share connections across SDKs; per-tenant scoping (rows
    # isolated by seed identity) is preserved.
    pool = create_mysql_connection_pool(config=mysql_config)

    try:
        # Build the SDK with MySQL backend (storage, tree store, and token store)
        builder = SdkBuilder(config=config, seed=seed)
        await builder.with_mysql_connection_pool(pool=pool)
        sdk = await builder.build()
        return sdk
    except Exception as error:
        logging.error(error)
        raise
# ANCHOR_END: init-sdk-mysql


async def init_sdk_server():
    # ANCHOR: init-sdk-server
    # Construct the seed using a mnemonic, entropy or passkey
    mnemonic = "<mnemonic words>"
    seed = Seed.MNEMONIC(mnemonic=mnemonic, passphrase=None)

    # Build a server-mode config: same as default_config(network) with
    # background_tasks_enabled = False. No periodic sync, no real-time sync
    # client, no leaf/token optimizer, no flashnet refunder, no lightning-
    # address recovery, no spark private-mode init.
    config = default_server_config(network=Network.MAINNET)
    config.api_key = "<breez api key>"

    try:
        # Typically server-mode SDKs are built per request and share
        # infrastructure (DB pool, REST chain service, SSP/Connection Manager)
        # across instances. Pass the shared resources via the builder.
        builder = SdkBuilder(config=config, seed=seed)
        await builder.with_default_storage(storage_dir="./.data")
        sdk = await builder.build()
        return sdk
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: init-sdk-server


async def server_mode_request_handler(sdk: BreezSdk):
    # ANCHOR: server-mode-request-handler
    # User-facing request handler: do not call sync_wallet here. Operations
    # that read from local storage (get_info, list_payments, etc.) do not
    # need a defensive sync. Call sync_wallet only from webhook handlers or
    # reconciliation jobs that need to observe an external state change.
    payment_method = ReceivePaymentMethod.BOLT11_INVOICE(
        description="<invoice description>",
        amount_sats=5_000,
        expiry_secs=3600,
        payment_hash=None,
    )
    response = await sdk.receive_payment(
        request=ReceivePaymentRequest(payment_method=payment_method)
    )

    # Always disconnect at the end of the request lifecycle to flush
    # outstanding storage writes.
    await sdk.disconnect()
    # ANCHOR_END: server-mode-request-handler
    return response.payment_request


async def server_mode_provisioning(sdk: BreezSdk):
    # ANCHOR: server-mode-provisioning
    # One-time setup when a wallet is first registered. The client-mode SDK
    # would normally apply the private-mode preset itself on first startup;
    # server-mode SDKs do not, so opt in once here via update_user_settings.
    await sdk.update_user_settings(
        request=UpdateUserSettingsRequest(spark_private_mode_enabled=True)
    )

    await sdk.disconnect()
    # ANCHOR_END: server-mode-provisioning


async def refund_pending_conversions(sdk: BreezSdk):
    # ANCHOR: refund-pending-conversions
    # The returned response reports how many were refunded and how many were
    # skipped (too young to recover).
    await sdk.refund_pending_conversions()
    # ANCHOR_END: refund-pending-conversions
