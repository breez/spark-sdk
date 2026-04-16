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
        for leaf in response.leaves:
            logging.debug(
                f"Leaf {leaf.leaf_id}: {leaf.value} sats "
                f"(exit cost: ~{leaf.estimated_cost} sats)"
            )
            for tx in leaf.transactions:
                if tx.csv_timelock_blocks is not None:
                    logging.debug(f"Timelock: wait {tx.csv_timelock_blocks} blocks")
                # tx.tx_hex: pre-signed Spark transaction
                # tx.cpfp_tx_hex: signed CPFP transaction — broadcast alongside parent

        if response.unverified_node_ids:
            logging.warning(
                f"Could not verify confirmation status for "
                f"{len(response.unverified_node_ids)} nodes"
            )
        # ANCHOR_END: prepare-unilateral-exit
    except Exception as error:
        logging.error(error)
        raise
