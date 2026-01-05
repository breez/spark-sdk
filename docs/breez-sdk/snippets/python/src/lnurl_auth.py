import logging
from breez_sdk_spark import (
    BreezSdk,
    InputType,
    LnurlAuthRequestDetails,
    LnurlCallbackStatus,
)


async def parse_lnurl_auth(sdk: BreezSdk):
    # ANCHOR: parse-lnurl-auth
    # LNURL-auth URL from a service
    # Can be in the form:
    # - lnurl1... (bech32 encoded)
    # - https://service.com/lnurl-auth?tag=login&k1=...
    lnurl_auth_url = "lnurl1..."

    try:
        input_type = await sdk.parse(lnurl_auth_url)
        if isinstance(input_type, InputType.LNURL_AUTH):
            request_data = input_type[0]
            logging.debug(f"Domain: {request_data.domain}")
            logging.debug(f"Action: {request_data.action}")

            # Show domain to user and ask for confirmation
            # This is important for security
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: parse-lnurl-auth


async def authenticate(sdk: BreezSdk, request_data: LnurlAuthRequestDetails):
    # ANCHOR: lnurl-auth
    # Perform LNURL authentication
    try:
        result = await sdk.lnurl_auth(request_data=request_data)

        if isinstance(result, LnurlCallbackStatus.OK):
            logging.debug("Authentication successful")
        elif isinstance(result, LnurlCallbackStatus.ERROR_STATUS):
            logging.debug(f"Authentication failed: {result.error_details.reason}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: lnurl-auth
