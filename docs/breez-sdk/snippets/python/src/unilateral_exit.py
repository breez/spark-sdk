import logging
from breez_sdk_spark import (
    BreezSdk,
    ListLeavesRequest,
    PrepareUnilateralExitRequest,
    UnilateralExitCpfpUtxo,
    UnilateralExitCpfpUtxoType,
)


async def list_leaves_for_exit(sdk: BreezSdk):
    try:
        # ANCHOR: list-leaves
        response = await sdk.list_leaves(
            request=ListLeavesRequest(min_value_sats=10_000)
        )

        for leaf in response.leaves:
            logging.info(f"Leaf {leaf.id}: {leaf.value} sats")
        # ANCHOR_END: list-leaves
    except Exception as error:
        logging.error(error)
        raise


async def prepare_exit(sdk: BreezSdk):
    try:
        # ANCHOR: prepare-unilateral-exit
        leaf_ids = ["leaf-id-1", "leaf-id-2"]

        response = await sdk.prepare_unilateral_exit(
            request=PrepareUnilateralExitRequest(
                fee_rate=2,
                leaf_ids=leaf_ids,
                utxos=[
                    UnilateralExitCpfpUtxo(
                        txid="your-utxo-txid",
                        vout=0,
                        value=50_000,
                        pubkey="your-compressed-pubkey-hex",
                        utxo_type=UnilateralExitCpfpUtxoType.P2WPKH,
                    )
                ],
                destination="bc1q...your-destination-address",
            )
        )

        # The response contains:
        # - response.leaves: transaction/PSBT pairs to sign and broadcast
        # - response.sweep_tx_hex: signed sweep transaction for the final step
        for leaf in response.leaves:
            for pair in leaf.tx_cpfp_psbts:
                if pair.csv_timelock_blocks is not None:
                    logging.info(f"Timelock: wait {pair.csv_timelock_blocks} blocks")
                # pair.parent_tx_hex: pre-signed Spark transaction
                # pair.child_psbt_hex: unsigned CPFP PSBT — sign with your UTXO key
        # ANCHOR_END: prepare-unilateral-exit
    except Exception as error:
        logging.error(error)
        raise
