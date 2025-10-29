import logging
from breez_sdk_spark import BreezSdk, CheckMessageRequest, SignMessageRequest


async def sign_message(sdk: BreezSdk):
    # ANCHOR: sign-message
    message = "<message to sign>"
    # Set to true to get a compact signature rather than a DER
    compact = True
    try:
        sign_message_request = SignMessageRequest(
            message=message, compact=compact
        )
        sign_message_response = await sdk.sign_message(request=sign_message_request)

        signature = sign_message_response.signature
        pubkey = sign_message_response.pubkey

        logging.debug(f"Pubkey: {pubkey}")
        logging.debug(f"Signature: {signature}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: sign-message


async def check_message(sdk: BreezSdk):
    # ANCHOR: check-message
    message = "<message>"
    pubkey = "<pubkey of signer>"
    signature = "<message signature>"
    try:
        check_message_request = CheckMessageRequest(
            message=message, pubkey=pubkey, signature=signature
        )
        check_message_response = await sdk.check_message(request=check_message_request)

        is_valid = check_message_response.is_valid

        logging.debug(f"Signature valid: {is_valid}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: check-message
