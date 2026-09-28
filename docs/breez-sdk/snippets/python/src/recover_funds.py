import logging
from breez_sdk_spark import (
    BreezSdk,
    CheckRecoverFundsRequest,
    CpfpFundingKind,
    CpfpInput,
    CpfpSigner,
    ExitLeafSelection,
    ExitTransactionStatus,
    GetInfoRequest,
    ImportUnilateralExitStateRequest,
    PrepareRecoverFundsRequest,
    PrepareRecoverFundsResponse,
    RecoverFundsRequest,
    RecoverFundsResponse,
    RecoveryMethod,
    RecoveryVerdict,
    SyncWalletRequest,
    single_key_cpfp_signer,
)


async def fetch_recoverable_funds(sdk: BreezSdk):
    try:
        # ANCHOR: recoverable-funds
        info = await sdk.get_info(request=GetInfoRequest(ensure_synced=False))

        if info.recoverable_funds_sats > 0:
            logging.debug(f"{info.recoverable_funds_sats} sats can be recovered on-chain")
        # ANCHOR_END: recoverable-funds
    except Exception as error:
        logging.error(error)
        raise


async def quote_recovery(sdk: BreezSdk) -> PrepareRecoverFundsResponse:
    try:
        # ANCHOR: prepare-recover-funds
        quote = await sdk.prepare_recover_funds(
            request=PrepareRecoverFundsRequest(
                fee_rate_sat_per_vbyte=2,
                funding_kind=CpfpFundingKind.P2WPKH(),
                destination="bc1q...your-destination-address",
                selection=ExitLeafSelection.RECOVERABLE_ONLY(),
            ),
        )

        if not quote.leaves:
            logging.debug("Nothing to recover")
            return quote
        for leaf in quote.leaves:
            logging.debug(f"{leaf.leaf_id}: {leaf.value_sats} sats, {leaf.method}")
        logging.debug(
            f"Recovering {quote.recoverable_value_sats} sats "
            f"for {quote.total_fee_sats} sats in fees"
        )
        if quote.funding is not None:
            logging.debug(f"Fund one UTXO of at least {quote.funding.single_utxo_sats} sats")
        # ANCHOR_END: prepare-recover-funds
        return quote
    except Exception as error:
        logging.error(error)
        raise


async def recover_cooperatively(sdk: BreezSdk, quote: PrepareRecoverFundsResponse):
    try:
        # ANCHOR: recover-cooperatively
        # A quote that asks for funding holds a unilateral exit: quote the
        # cooperative leaves alone to recover them without it.
        if quote.funding is not None:
            leaf_ids = [
                leaf.leaf_id
                for leaf in quote.leaves
                if leaf.method == RecoveryMethod.COOPERATIVE
            ]
            if not leaf_ids:
                return
            quote = await sdk.prepare_recover_funds(
                request=PrepareRecoverFundsRequest(
                    fee_rate_sat_per_vbyte=quote.fee_rate_sat_per_vbyte,
                    destination=quote.destination,
                    selection=ExitLeafSelection.SPECIFIC(leaf_ids=leaf_ids),
                ),
            )
        response = await sdk.recover_funds(
            request=RecoverFundsRequest(prepared=quote),
            signer=None,
        )

        # Keep the whole response: check_recover_funds follows the recovery from it.
        for tx in response.transactions:
            logging.debug(f"Broadcast {tx.txid}: {tx.tx_hex}")
        for failure in response.failed:
            logging.debug(f"Leaf {failure.leaf_id} was not recovered: {failure.error}")
        # ANCHOR_END: recover-cooperatively
    except Exception as error:
        logging.error(error)
        raise


async def recover_with_funding(
    sdk: BreezSdk, quote: PrepareRecoverFundsResponse
) -> RecoverFundsResponse:
    try:
        # ANCHOR: recover-funds
        secret_key_bytes = bytes.fromhex("your-secret-key-hex")
        signer = single_key_cpfp_signer(secret_key_bytes=secret_key_bytes)

        response = await sdk.recover_funds(
            request=RecoverFundsRequest(
                prepared=quote,
                funding_inputs=[
                    CpfpInput.P2WPKH(  # type: ignore[list-item]
                        txid="your-utxo-txid",
                        vout=0,
                        value_sats=50_000,
                        pubkey="your-compressed-pubkey-hex",
                    )
                ],
            ),
            signer=signer,
        )

        # Keep the whole response: check_recover_funds follows the recovery from it.
        for tx in response.transactions:
            if tx.csv_timelock_blocks is not None:
                logging.debug(
                    f"{tx.txid}: wait {tx.csv_timelock_blocks} blocks after its parents confirm"
                )
        # ANCHOR_END: recover-funds
        return response
    except Exception as error:
        logging.error(error)
        raise


async def check_recovery(sdk: BreezSdk, stored: RecoverFundsResponse):
    try:
        # ANCHOR: check-recover-funds
        checked = await sdk.check_recover_funds(
            request=CheckRecoverFundsRequest(recovery=stored)
        )

        # Store this one in place of the one you had.
        recovery = checked.recovery

        if isinstance(checked.verdict, RecoveryVerdict.VALID):
            for tx in recovery.transactions:
                if isinstance(tx.status, ExitTransactionStatus.READY):
                    logging.debug(f"ready to broadcast: {tx.txid}")
        elif isinstance(checked.verdict, RecoveryVerdict.DONE):
            logging.debug("Every transaction confirmed: the recovery is done")
        elif isinstance(checked.verdict, RecoveryVerdict.REDO):
            # Quote and build again, naming the same leaves. Pass
            # recovery.funding_inputs back and the SDK follows them to whatever
            # they have become.
            logging.debug(f"Build the recovery again: {checked.verdict.reason}")
        # ANCHOR_END: check-recover-funds
    except Exception as error:
        logging.error(error)
        raise


async def back_up_exit_state(sdk: BreezSdk) -> str:
    try:
        # ANCHOR: export-exit-state
        exported = await sdk.export_unilateral_exit_state()

        # Keep the state somewhere the wallet's own storage cannot take with it.
        logging.debug(f"Exit state is {len(exported.exit_state)} bytes")
        # ANCHOR_END: export-exit-state
        return exported.exit_state
    except Exception as error:
        logging.error(error)
        raise


async def restore_exit_state(sdk: BreezSdk, exit_state: str):
    try:
        # ANCHOR: import-exit-state
        imported = await sdk.import_unilateral_exit_state(
            request=ImportUnilateralExitStateRequest(exit_state=exit_state)
        )

        logging.debug(
            f"Imported {imported.imported_leaves} leaves, "
            f"skipped {imported.skipped_foreign_leaves}"
        )
        # ANCHOR_END: import-exit-state
    except Exception as error:
        logging.error(error)
        raise


async def collect_exit_data(sdk: BreezSdk):
    try:
        # ANCHOR: sync-exit-data
        # With automatic collection off, an explicit sync is what collects the data
        # a unilateral exit needs, and it waits for the collection to finish. Needs
        # the Spark operators reachable, so run it on a schedule rather than at the
        # moment an exit is needed.
        await sdk.sync_wallet(request=SyncWalletRequest())
        # ANCHOR_END: sync-exit-data
    except Exception as error:
        logging.error(error)
        raise


# ANCHOR: custom-cpfp-signer
class MyFundingSigner(CpfpSigner):
    async def sign_psbt(self, psbt_bytes: bytes) -> bytes:
        return sign_with_funding_keys(psbt_bytes)


def sign_with_funding_keys(psbt_bytes: bytes) -> bytes:
    return psbt_bytes
# ANCHOR_END: custom-cpfp-signer
