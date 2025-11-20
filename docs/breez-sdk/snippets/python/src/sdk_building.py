import logging
import typing
from breez_sdk_spark import (
    default_config,
    Network,
    ProvisionalPayment,
    SdkBuilder,
    Seed,
    ListPaymentsRequest,
    Payment,
    PaymentMetadata,
    UpdateDepositPayload,
    UnversionedRecordChange,
    Record,
    ChainApiType,
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
    await builder.with_key_set(
        key_set_type=key_set_type,
        use_address_index=use_address_index,
        account_number=optional_account_number,
    )
    # ANCHOR_END: with-key-set


# ANCHOR: with-storage
class Storage(typing.Protocol):
    def delete_cached_item(self, key: "str"):
        raise NotImplementedError
    def get_cached_item(self, key: "str"):
        raise NotImplementedError
    def set_cached_item(self, key: "str", value: "str"):
        raise NotImplementedError
    def list_payments(self, request: "ListPaymentsRequest"):
        raise NotImplementedError
    def insert_payment(self, payment: "Payment"):
        raise NotImplementedError
    def set_payment_metadata(self, payment_id: "str", metadata: "PaymentMetadata"):
        raise NotImplementedError
    def get_payment_by_id(self, id: "str"):
        raise NotImplementedError
    def get_payment_by_invoice(self, invoice: "str"):
        raise NotImplementedError
    def add_deposit(self, txid: "str", vout: "int", amount_sats: "int"):
        raise NotImplementedError
    def delete_deposit(self, txid: "str", vout: "int"):
        raise NotImplementedError
    def list_deposits(
        self,
    ):
        raise NotImplementedError
    def update_deposit(self, txid: "str", vout: "int", payload: "UpdateDepositPayload"):
        raise NotImplementedError
# ANCHOR_END: with-storage


# ANCHOR: with-sync-storage
class SyncStorage(typing.Protocol):
    def add_outgoing_change(self, record: "UnversionedRecordChange"):
        raise NotImplementedError
    def complete_outgoing_sync(self, record: "Record"):
        raise NotImplementedError
    def get_pending_outgoing_changes(self, limit: "int"):
        raise NotImplementedError
    def get_last_revision(
        self,
    ):
        raise NotImplementedError
    def insert_incoming_records(self, records: "typing.List[Record]"):
        raise NotImplementedError
    def delete_incoming_record(self, record: "Record"):
        raise NotImplementedError
    def rebase_pending_outgoing_records(self, revision: "int"):
        raise NotImplementedError
    def get_incoming_records(self, limit: "int"):
        raise NotImplementedError
    def get_latest_outgoing_change(
        self,
    ):
        raise NotImplementedError
    def update_record_from_incoming(self, record: "Record"):
        raise NotImplementedError
# ANCHOR_END: with-sync-storage


# ANCHOR: with-rest-client
class RestClient(typing.Protocol):
    def get_request(self, url: "str", headers: "typing.Optional[dict[str, str]]"):
        raise NotImplementedError
    def post_request(
        self,
        url: "str",
        headers: "typing.Optional[dict[str, str]]",
        body: "typing.Optional[str]",
    ):
        raise NotImplementedError
    def delete_request(
        self,
        url: "str",
        headers: "typing.Optional[dict[str, str]]",
        body: "typing.Optional[str]",
    ):
        raise NotImplementedError
# ANCHOR_END: with-rest-client


# ANCHOR: with-fiat-service
class FiatService(typing.Protocol):
    def fetch_fiat_currencies(
        self,
    ):
        raise NotImplementedError
    def fetch_fiat_rates(
        self,
    ):
        raise NotImplementedError
# ANCHOR_END: with-fiat-service


# ANCHOR: with-chain-service
class BitcoinChainService(typing.Protocol):
    def get_address_utxos(self, address: "str"):
        raise NotImplementedError
    def get_transaction_status(self, txid: "str"):
        raise NotImplementedError
    def get_transaction_hex(self, txid: "str"):
        raise NotImplementedError
    def broadcast_transaction(self, tx: "str"):
        raise NotImplementedError
# ANCHOR_END: with-chain-service


# ANCHOR: with-payment-observer
class PaymentObserver(typing.Protocol):
    async def before_send(self, payments: typing.List["ProvisionalPayment"]):
        raise NotImplementedError
# ANCHOR_END: with-payment-observer
