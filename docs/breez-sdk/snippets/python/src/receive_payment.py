import logging
from breez_sdk_spark import BreezSdk, PrepareReceivePaymentRequest, ReceivePaymentMethod, PrepareReceivePaymentResponse, ReceivePaymentRequest


def prepare_receive_lightning(sdk: BreezSdk):
    # ANCHOR: prepare-receive-payment-lightning
    try:
        description = "<invoice description>"
        # Optionally set the invoice amount you wish the payer to send
        optional_amount_sats = 5_000
        payment_method = ReceivePaymentMethod.BOLT11_INVOICE(
            description=description,
            amount_sats=optional_amount_sats
        )
        prepare_request = PrepareReceivePaymentRequest(
            payment_method=payment_method
        )
        prepare_response = sdk.prepare_receive_payment(prepare_request=prepare_request)

        receive_fee_sats = prepare_response.fee_sats
        logging.debug(f"Fees: {receive_fee_sats} sats")
        return prepare_response
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: prepare-receive-payment-lightning

def prepare_receive_onchain(sdk: BreezSdk):
    # ANCHOR: prepare-receive-payment-onchain
    try:
        prepare_request = PrepareReceivePaymentRequest(
            payment_method=ReceivePaymentMethod.BITCOIN_ADDRESS
        )
        prepare_response = sdk.prepare_receive_payment(prepare_request=prepare_request)

        receive_fee_sats = prepare_response.fee_sats
        logging.debug(f"Fees: {receive_fee_sats} sats")
        return prepare_response
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: prepare-receive-payment-onchain

def prepare_receive_spark(sdk: BreezSdk):
    # ANCHOR: prepare-receive-payment-spark
    try:
        prepare_request = PrepareReceivePaymentRequest(
            payment_method=ReceivePaymentMethod.SPARK_ADDRESS
        )
        prepare_response = sdk.prepare_receive_payment(prepare_request=prepare_request)

        receive_fee_sats = prepare_response.fee_sats
        logging.debug(f"Fees: {receive_fee_sats} sats")
        return prepare_response
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: prepare-receive-payment-spark

async def receive_payment(sdk: BreezSdk, prepare_response: PrepareReceivePaymentResponse):
    # ANCHOR: receive-payment
    try:
        request = ReceivePaymentRequest(
            prepare_response=prepare_response,
        )
        response = await sdk.receive_payment(request=request)
        payment_request = response.payment_request
        logging.debug(f"Payment Request: {payment_request}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: receive-payment
