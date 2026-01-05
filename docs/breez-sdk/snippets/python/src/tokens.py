# pylint: disable=duplicate-code
import logging
from breez_sdk_spark import (
    BreezSdk,
    GetInfoRequest,
    PrepareSendPaymentRequest,
    ReceivePaymentMethod,
    ReceivePaymentRequest,
    SendPaymentRequest,
    SendPaymentMethod,
    GetTokensMetadataRequest,
    TokenConversionOptions,
    TokenConversionType,
)


async def fetch_token_balances(sdk: BreezSdk):
    # ANCHOR: fetch-token-balances
    try:
        # ensure_synced: True will ensure the SDK is synced with the Spark network
        # before returning the balance
        info = await sdk.get_info(request=GetInfoRequest(ensure_synced=False))

        # Token balances are a map of token identifier to balance
        token_balances = info.token_balances
        for token_id, token_balance in token_balances.items():
            print(f"Token ID: {token_id}")
            print(f"Balance: {token_balance.balance}")
            print(f"Name: {token_balance.token_metadata.name}")
            print(f"Ticker: {token_balance.token_metadata.ticker}")
            print(f"Decimals: {token_balance.token_metadata.decimals}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: fetch-token-balances

async def fetch_token_metadata(sdk: BreezSdk):
    # ANCHOR: fetch-token-metadata
    try:
        response = await sdk.get_tokens_metadata(
            request=GetTokensMetadataRequest(
                token_identifiers=["<token identifier 1>", "<token identifier 2>"]
                )
            )

        tokens_metadata = response.tokens_metadata
        for token_metadata in tokens_metadata:
            print(f"Token ID: {token_metadata.identifier}")
            print(f"Name: {token_metadata.name}")
            print(f"Ticker: {token_metadata.ticker}")
            print(f"Decimals: {token_metadata.decimals}")
            print(f"Max Supply: {token_metadata.max_supply}")
            print(f"Is Freezable: {token_metadata.is_freezable}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: fetch-token-metadata


async def receive_token_payment_spark_invoice(sdk: BreezSdk):
    # ANCHOR: receive-token-payment-spark-invoice
    try:
        token_identifier = "<token identifier>"
        optional_description = "<invoice description>"
        optional_amount = 5_000
        optional_expiry_time_seconds = 1716691200
        optional_sender_public_key = "<sender public key>"

        request = ReceivePaymentRequest(
            payment_method=ReceivePaymentMethod.SPARK_INVOICE(
                token_identifier=token_identifier,
                description=optional_description,
                amount=optional_amount,
                expiry_time=optional_expiry_time_seconds,
                sender_public_key=optional_sender_public_key,
            )
        )
        response = await sdk.receive_payment(request=request)

        payment_request = response.payment_request
        print(f"Payment request: {payment_request}")
        receive_fee = response.fee
        print(f"Fees: {receive_fee} token base units")
        return response
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: receive-token-payment-spark-invoice


async def send_token_payment(sdk: BreezSdk):
    # ANCHOR: send-token-payment
    try:
        payment_request = "<spark address or invoice>"
        # Token identifier must match the invoice in case it specifies one.
        token_identifier = "<token identifier>"
        # Set the amount of tokens you wish to send.
        optional_amount = 1_000
        # Optionally set to use Bitcoin funds to pay via token conversion
        optional_token_conversion_options = TokenConversionOptions(
            conversion_type=TokenConversionType.FROM_BITCOIN(),
            max_slippage_bps=50,
        )

        prepare_response = await sdk.prepare_send_payment(
            request=PrepareSendPaymentRequest(
                payment_request=payment_request,
                amount=optional_amount,
                token_identifier=token_identifier,
                token_conversion_options=optional_token_conversion_options,
            )
        )

        # If the fees are acceptable, continue to send the token payment
        if prepare_response.token_conversion_fee is not None:
            token_conversion_fee = prepare_response.token_conversion_fee
            logging.debug(
                f"Estimated token conversion fee: {token_conversion_fee} sats"
            )
        if isinstance(prepare_response.payment_method, SendPaymentMethod.SPARK_ADDRESS):
            print(f"Token ID: {prepare_response.payment_method.token_identifier}")
            print(f"Fees: {prepare_response.payment_method.fee} token base units")
        if isinstance(prepare_response.payment_method, SendPaymentMethod.SPARK_INVOICE):
            print(f"Token ID: {prepare_response.payment_method.token_identifier}")
            print(f"Fees: {prepare_response.payment_method.fee} token base units")

        # Send the token payment
        send_response = await sdk.send_payment(
            request=SendPaymentRequest(
                prepare_response=prepare_response,
                options=None,
            )
        )
        payment = send_response.payment
        print(f"Payment: {payment}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: send-token-payment
