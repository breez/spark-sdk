# pylint: disable=duplicate-code
import logging
from breez_sdk_spark import (
    BreezSdk,
    FetchConversionLimitsRequest,
    GetInfoRequest,
    GetTokensMetadataRequest,
    PrepareSendPaymentRequest,
    ReceivePaymentMethod,
    ReceivePaymentRequest,
    SendPaymentMethod,
    SendPaymentRequest,
    ConversionOptions,
    ConversionType,
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
        # Optionally set the expiry UNIX timestamp in seconds
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

        prepare_response = await sdk.prepare_send_payment(
            request=PrepareSendPaymentRequest(
                payment_request=payment_request,
                amount=optional_amount,
                token_identifier=token_identifier,
            )
        )

        # If the fees are acceptable, continue to send the token payment
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


async def fetch_conversion_limits(sdk: BreezSdk):
    # ANCHOR: fetch-token-conversion-limits
    try:
        # Fetch limits for converting Bitcoin to a token
        from_bitcoin_response = await sdk.fetch_conversion_limits(
            request=FetchConversionLimitsRequest(
                conversion_type=ConversionType.FROM_BITCOIN(),
                token_identifier="<token identifier>",
            )
        )

        if from_bitcoin_response.min_from_amount is not None:
            print(f"Minimum BTC to convert: {from_bitcoin_response.min_from_amount} sats")
        if from_bitcoin_response.min_to_amount is not None:
            print(f"Minimum tokens to receive: {from_bitcoin_response.min_to_amount} base units")

        # Fetch limits for converting a token to Bitcoin
        to_bitcoin_response = await sdk.fetch_conversion_limits(
            request=FetchConversionLimitsRequest(
                conversion_type=ConversionType.TO_BITCOIN(
                    from_token_identifier="<token identifier>"
                ),
                token_identifier=None,
            )
        )

        if to_bitcoin_response.min_from_amount is not None:
            print(f"Minimum tokens to convert: {to_bitcoin_response.min_from_amount} base units")
        if to_bitcoin_response.min_to_amount is not None:
            print(f"Minimum BTC to receive: {to_bitcoin_response.min_to_amount} sats")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: fetch-token-conversion-limits


async def prepare_send_payment_token_conversion(sdk: BreezSdk):
    # ANCHOR: prepare-send-payment-token-conversion
    try:
        payment_request = "<spark address or invoice>"
        # Token identifier must match the invoice in case it specifies one.
        token_identifier = "<token identifier>"
        # Set the amount of tokens you wish to send.
        optional_amount = 1_000
        # Set to use Bitcoin funds to pay via token conversion
        optional_max_slippage_bps = 50
        optional_completion_timeout_secs = 30
        conversion_options = ConversionOptions(
            conversion_type=ConversionType.FROM_BITCOIN(),
            max_slippage_bps=optional_max_slippage_bps,
            completion_timeout_secs=optional_completion_timeout_secs,
        )

        prepare_response = await sdk.prepare_send_payment(
            request=PrepareSendPaymentRequest(
                payment_request=payment_request,
                amount=optional_amount,
                token_identifier=token_identifier,
                conversion_options=conversion_options,
            )
        )

        # If the fees are acceptable, continue to send the token payment
        if prepare_response.conversion_estimate is not None:
            conversion_estimate = prepare_response.conversion_estimate
            logging.debug(
                f"Estimated conversion amount: {conversion_estimate.amount} sats"
            )
            logging.debug(
                f"Estimated conversion fee: {conversion_estimate.fee} sats"
            )
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: prepare-send-payment-token-conversion
