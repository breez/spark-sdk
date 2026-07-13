import logging
from breez_sdk_spark import (
    BreezSdk,
    CpfpFundingKind,
    CpfpInput,
    CpfpSigner,
    ExitLeafSelection,
    PrepareUnilateralExitRequest,
    PrepareUnilateralExitResponse,
    UnilateralExitRequest,
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


# ANCHOR: custom-cpfp-signer
class CustomCpfpSigner(CpfpSigner):
    async def sign_psbt(self, psbt_bytes: bytes) -> bytes:
        return sign_psbt_with_your_keys(psbt_bytes)


def sign_psbt_with_your_keys(psbt_bytes: bytes) -> bytes:
    raise NotImplementedError("Sign the PSBT's non-finalized inputs with your keys")
# ANCHOR_END: custom-cpfp-signer
