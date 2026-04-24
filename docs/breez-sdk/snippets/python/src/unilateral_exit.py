import logging
from breez_sdk_spark import (
    BreezSdk,
    ListLeavesRequest,
    PrepareUnilateralExitRequest,
    SingleKeySigner,
    UnilateralExitCpfpInput,
)


async def list_leaves_for_exit(sdk: BreezSdk):
    try:
        # ANCHOR: list-leaves
        response = await sdk.list_leaves(
            request=ListLeavesRequest(min_value_sats=10_000)
        )

        for leaf in response.leaves:
            logging.debug(f"Leaf {leaf.id}: {leaf.value} sats")
        # ANCHOR_END: list-leaves
    except Exception as error:
        logging.error(error)
        raise


async def prepare_exit(sdk: BreezSdk):
    try:
        # ANCHOR: prepare-unilateral-exit
        leaf_ids = ["leaf-id-1", "leaf-id-2"]

        # Create a signer from your UTXO private key (32-byte secret key)
        secret_key_bytes = bytes.fromhex("your-secret-key-hex")
        signer = SingleKeySigner(secret_key_bytes=secret_key_bytes)

        response = await sdk.prepare_unilateral_exit(
            request=PrepareUnilateralExitRequest(
                fee_rate=2,
                leaf_ids=leaf_ids,
                inputs=[
                    UnilateralExitCpfpInput.P2WPKH(  # type: ignore[list-item]
                        txid="your-utxo-txid",
                        vout=0,
                        value=50_000,
                        pubkey="your-compressed-pubkey-hex",
                    )
                ],
                destination="bc1q...your-destination-address",
            ),
            signer=signer,
        )

        # The response contains signed transactions ready to broadcast:
        # - response.leaves: parent/child transaction pairs
        # - response.sweep_tx_hex: signed sweep transaction for the final step
        # Change from CPFP fee-bumping always goes back to the first input's address.
        for leaf in response.leaves:
            for pair in leaf.tx_cpfp_pairs:
                if pair.csv_timelock_blocks is not None:
                    logging.debug(f"Timelock: wait {pair.csv_timelock_blocks} blocks")
                # pair.parent_tx_hex: pre-signed Spark transaction
                # pair.child_tx_hex: signed CPFP transaction — broadcast alongside parent
        # ANCHOR_END: prepare-unilateral-exit
    except Exception as error:
        logging.error(error)
        raise
