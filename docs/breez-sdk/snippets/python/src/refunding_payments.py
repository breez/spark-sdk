import logging
from breez_sdk_spark import (
    BreezSdk,
    ListUnclaimedDepositsRequest,
    ClaimDepositRequest,
    RefundDepositRequest,
    Fee,
    DepositClaimError,
)


async def list_unclaimed_deposits(sdk: BreezSdk):
    # ANCHOR: list-unclaimed-deposits
    try:
        request = ListUnclaimedDepositsRequest()
        response = await sdk.list_unclaimed_deposits(request=request)

        for deposit in response.deposits:
            logging.info(f"Unclaimed deposit: {deposit.txid}:{deposit.vout}")
            logging.info(f"Amount: {deposit.amount_sats} sats")

            if deposit.claim_error:
                if isinstance(
                    deposit.claim_error, DepositClaimError.MAX_DEPOSIT_CLAIM_FEE_EXCEEDED
                ):
                    max_fee_str = (
                        f"{deposit.claim_error.max_fee} sats"
                        if deposit.claim_error.max_fee is not None
                        else "none"
                    )
                    logging.info(
                        f"Claim failed: Fee exceeded. Max: {max_fee_str}, "
                        f"Required: {deposit.claim_error.required_fee}"
                    )
                elif isinstance(deposit.claim_error, DepositClaimError.MISSING_UTXO):
                    logging.info("Claim failed: UTXO not found")
                elif isinstance(deposit.claim_error, DepositClaimError.GENERIC):
                    logging.info(f"Claim failed: {deposit.claim_error.message}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: list-unclaimed-deposits


async def handle_fee_exceeded(sdk: BreezSdk, deposit):
    # ANCHOR: handle-fee-exceeded
    try:
        if isinstance(
            deposit.claim_error, DepositClaimError.MAX_DEPOSIT_CLAIM_FEE_EXCEEDED
        ):
            required_fee = deposit.claim_error.required_fee

            # Show UI to user with the required fee and get approval
            user_approved = True  # Replace with actual user approval logic

            if user_approved:
                claim_request = ClaimDepositRequest(
                    txid=deposit.txid,
                    vout=deposit.vout,
                    max_fee=Fee.FIXED(amount=required_fee),
                )
                await sdk.claim_deposit(request=claim_request)
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: handle-fee-exceeded


async def claim_deposit(sdk: BreezSdk):
    # ANCHOR: claim-deposit
    try:
        txid = "your_deposit_txid"
        vout = 0

        # Set a higher max fee to retry claiming
        max_fee = Fee.FIXED(amount=5_000)

        request = ClaimDepositRequest(txid=txid, vout=vout, max_fee=max_fee)

        response = await sdk.claim_deposit(request=request)
        logging.info(f"Deposit claimed successfully. Payment: {response.payment}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: claim-deposit


async def refund_deposit(sdk: BreezSdk):
    # ANCHOR: refund-deposit
    try:
        txid = "your_deposit_txid"
        vout = 0
        destination_address = "bc1qexample..."  # Your Bitcoin address

        # Set the fee for the refund transaction using a rate
        fee = Fee.RATE(sat_per_vbyte=5)
        # or using a fixed amount
        #fee = Fee.FIXED(amount=500)

        request = RefundDepositRequest(
            txid=txid, vout=vout, destination_address=destination_address, fee=fee
        )

        response = await sdk.refund_deposit(request=request)
        logging.info("Refund transaction created:")
        logging.info(f"Transaction ID: {response.tx_id}")
        logging.info(f"Transaction hex: {response.tx_hex}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: refund-deposit

async def recommended_feeds(sdk: BreezSdk):
    # ANCHOR: recommended-fees
    response = await sdk.recommended_fees()
    logging.info(f"Fastest fee: {response.fastest_fee} sats/vByte")
    logging.info(f"Half-hour fee: {response.half_hour_fee} sats/vByte")
    logging.info(f"Hour fee: {response.hour_fee} sats/vByte")
    logging.info(f"Economy fee: {response.economy_fee} sats/vByte")
    logging.info(f"Minimum fee: {response.minimum_fee} sats/vByte")
    # ANCHOR_END: recommended-fees
