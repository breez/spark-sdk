import logging
from breez_sdk_spark import (
    BreezSdk,
    CheckUnilateralExitRequest,
    CpfpFundingKind,
    CpfpInput,
    CpfpSigner,
    ExitLeafSelection,
    ExitTransactionStatus,
    ImportUnilateralExitStateRequest,
    PrepareUnilateralExitRequest,
    PrepareUnilateralExitResponse,
    UnilateralExitRequest,
    UnilateralExitResponse,
    UnilateralExitVerdict,
    single_key_cpfp_signer,
)


async def quote_exit(sdk: BreezSdk):
    try:
        # ANCHOR: prepare-unilateral-exit
        quote = await sdk.prepare_unilateral_exit(
            request=PrepareUnilateralExitRequest(
                fee_rate_sat_per_vbyte=2,
                funding_kind=CpfpFundingKind.P2WPKH(),
                destination="bc1q...your-destination-address",
                selection=ExitLeafSelection.AUTO(),
            ),
        )

        logging.debug(
            f"Recovering {quote.recoverable_value_sat} sats "
            f"for {quote.total_fee_sat} sats in fees"
        )
        logging.debug(f"Fund a single UTXO of at least {quote.single_utxo_funding_sat} sats")
        # ANCHOR_END: prepare-unilateral-exit
        return quote
    except Exception as error:
        logging.error(error)
        raise


async def build_exit(sdk: BreezSdk, quote: PrepareUnilateralExitResponse):
    try:
        # ANCHOR: unilateral-exit
        secret_key_bytes = bytes.fromhex("your-secret-key-hex")
        signer = single_key_cpfp_signer(secret_key_bytes=secret_key_bytes)

        response = await sdk.unilateral_exit(
            request=UnilateralExitRequest(
                prepared=quote,
                funding_inputs=[
                    CpfpInput.P2WPKH(  # type: ignore[list-item]
                        txid="your-utxo-txid",
                        vout=0,
                        value=50_000,
                        pubkey="your-compressed-pubkey-hex",
                    )
                ],
            ),
            signer=signer,
        )

        for tx in response.transactions:
            if tx.csv_timelock_blocks is not None:
                logging.debug(
                    f"{tx.txid}: wait {tx.csv_timelock_blocks} blocks after its parents confirm"
                )
        # ANCHOR_END: unilateral-exit
    except Exception as error:
        logging.error(error)
        raise


async def check_exit(sdk: BreezSdk, stored: UnilateralExitResponse):
    try:
        # ANCHOR: check-unilateral-exit
        checked = await sdk.check_unilateral_exit(
            request=CheckUnilateralExitRequest(exit=stored)
        )

        # Store this one in place of the one you had.
        exit = checked.exit

        if isinstance(checked.verdict, UnilateralExitVerdict.VALID):
            for tx in exit.transactions:
                if isinstance(tx.status, ExitTransactionStatus.READY):
                    logging.debug(f"ready to broadcast: {tx.txid}")
        elif isinstance(checked.verdict, UnilateralExitVerdict.DONE):
            logging.debug(f"The exit finished: {exit.recoverable_value_sat} sats recovered")
        elif isinstance(checked.verdict, UnilateralExitVerdict.REDO):
            # Quote and build again, naming the same leaves. Pass exit.funding_inputs
            # back and the SDK follows them to whatever they have become.
            logging.debug(f"Build the exit again: {checked.verdict.reason}")
        # ANCHOR_END: check-unilateral-exit
    except Exception as error:
        logging.error(error)
        raise


async def export_exit_state(sdk: BreezSdk) -> str:
    try:
        # ANCHOR: export-unilateral-exit-state
        exported = await sdk.export_unilateral_exit_state()

        # Keep the state somewhere the wallet's own storage cannot take with it.
        logging.debug(f"Exit state is {len(exported.exit_state)} bytes")
        # ANCHOR_END: export-unilateral-exit-state
        return exported.exit_state
    except Exception as error:
        logging.error(error)
        raise


async def import_exit_state(sdk: BreezSdk, exit_state: str):
    try:
        # ANCHOR: import-unilateral-exit-state
        imported = await sdk.import_unilateral_exit_state(
            request=ImportUnilateralExitStateRequest(exit_state=exit_state)
        )

        logging.debug(
            f"Imported {imported.imported_leaves} leaves, "
            f"skipped {imported.skipped_foreign_leaves}"
        )
        # ANCHOR_END: import-unilateral-exit-state
    except Exception as error:
        logging.error(error)
        raise


# ANCHOR: custom-cpfp-signer
class CustomCpfpSigner(CpfpSigner):
    async def sign_psbt(self, psbt_bytes: bytes) -> bytes:
        return sign_psbt_with_your_keys(psbt_bytes)


def sign_psbt_with_your_keys(psbt_bytes: bytes) -> bytes:
    raise NotImplementedError("Sign the PSBT's non-finalized inputs with your keys")
# ANCHOR_END: custom-cpfp-signer
