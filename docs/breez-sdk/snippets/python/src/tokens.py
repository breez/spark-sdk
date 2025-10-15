import logging
from breez_sdk_spark import (
    BreezSdk,
    GetInfoRequest,
    PrepareSendPaymentRequest,
    SendPaymentRequest,
    SendPaymentMethod,
    GetTokensMetadataRequest,
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


async def send_token_payment(sdk: BreezSdk):
    # ANCHOR: send-token-payment
    try:
        payment_request = "<spark address>"
        # The token identifier (e.g., asset ID or token contract)
        token_identifier = "<token identifier>"
        # Set the amount of tokens you wish to send
        amount = 1_000

        prepare_response = await sdk.prepare_send_payment(
            request=PrepareSendPaymentRequest(
                payment_request=payment_request,
                amount=amount,
                token_identifier=token_identifier,
            )
        )

        # If the fees are acceptable, continue to send the token payment
        if isinstance(prepare_response.payment_method, SendPaymentMethod.SPARK_ADDRESS):
            print(f"Token ID: {prepare_response.payment_method.token_identifier}")
            print(f"Fees: {prepare_response.payment_method.fee} sats")

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
