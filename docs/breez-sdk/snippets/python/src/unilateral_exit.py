import logging
from breez_sdk_spark import (
    BreezSdk,
    PrepareUnilateralExitRequest,
    SingleKeySigner,
    UnilateralExitCpfpInput,
)


async def prepare_exit(sdk: BreezSdk):
    try:
        # ANCHOR: prepare-unilateral-exit
        # Create a signer from your UTXO private key (32-byte secret key)
        secret_key_bytes = bytes.fromhex("your-secret-key-hex")
        signer = SingleKeySigner(secret_key_bytes=secret_key_bytes)

        response = await sdk.prepare_unilateral_exit(
            request=PrepareUnilateralExitRequest(
                fee_rate=2,
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

        # The SDK automatically selects which leaves are profitable to exit.
        for leaf in response.selected_leaves:
            logging.debug(
                f"Leaf {leaf.id}: {leaf.value} sats "
                f"(exit cost: ~{leaf.estimated_cost} sats)"
            )

        # The response contains signed transactions ready to broadcast:
        # - response.transactions: parent/child transaction pairs per leaf
        # - response.sweep_tx_hex: signed sweep transaction for the final step
        # Change from CPFP fee-bumping always goes back to the first input's address.
        for leaf in response.transactions:
            for pair in leaf.tx_cpfp_pairs:
                if pair.csv_timelock_blocks is not None:
                    logging.debug(f"Timelock: wait {pair.csv_timelock_blocks} blocks")
                # pair.parent_tx_hex: pre-signed Spark transaction
                # pair.child_tx_hex: signed CPFP transaction — broadcast alongside parent
        # ANCHOR_END: prepare-unilateral-exit
    except Exception as error:
        logging.error(error)
        raise
