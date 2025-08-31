import logging
from breez_sdk_spark import BreezSdk, ListUnclaimedDepositsRequest, ClaimDepositRequest, RefundDepositRequest, Fee, FeeType, DepositClaimError

async def list_unclaimed_deposits(sdk: BreezSdk):
    # ANCHOR: list-unclaimed-deposits
    try:
        request = ListUnclaimedDepositsRequest()
        response = await sdk.list_unclaimed_deposits(request=request)
        
        for deposit in response.deposits:
            logging.info(f"Unclaimed deposit: {deposit.txid}:{deposit.vout}")
            logging.info(f"Amount: {deposit.amount_sats} sats")
            
            if deposit.claim_error:
                if isinstance(deposit.claim_error, DepositClaimError.DEPOSIT_CLAIM_FEE_EXCEEDED):
                    logging.info(f"Claim failed: Fee exceeded. Max: {deposit.claim_error.max_fee}, Actual: {deposit.claim_error.actual_fee}")
                elif isinstance(deposit.claim_error, DepositClaimError.MISSING_UTXO):
                    logging.info("Claim failed: UTXO not found")
                elif isinstance(deposit.claim_error, DepositClaimError.GENERIC):
                    logging.info(f"Claim failed: {deposit.claim_error.message}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: list-unclaimed-deposits

async def claim_deposit(sdk: BreezSdk):
    # ANCHOR: claim-deposit
    try:
        txid = "your_deposit_txid"
        vout = 0
        
        # Set a higher max fee to retry claiming
        max_fee = Fee(fee_type=FeeType.ABSOLUTE(fee_sat=5000))  # 5000 sats
        
        request = ClaimDepositRequest(
            txid=txid,
            vout=vout,
            max_fee=max_fee
        )
        
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
        
        # Set the fee for the refund transaction
        fee = Fee(fee_type=FeeType.ABSOLUTE, fee_sat=500)  # 500 sats
        
        request = RefundDepositRequest(
            txid=txid,
            vout=vout,
            destination_address=destination_address,
            fee=fee
        )
        
        response = await sdk.refund_deposit(request=request)
        logging.info("Refund transaction created:")
        logging.info(f"Transaction ID: {response.tx_id}")
        logging.info(f"Transaction hex: {response.tx_hex}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: refund-deposit
