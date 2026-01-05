from breez_sdk_spark import *


def parse_lnurl_auth(sdk: BreezSdk):
    # ANCHOR: parse-lnurl-auth
    # LNURL-auth URL from a service
    # Can be in the form:
    # - lnurl1... (bech32 encoded)
    # - https://service.com/lnurl-auth?tag=login&k1=...
    lnurl_auth_url = "lnurl1..."

    input_type = sdk.parse(lnurl_auth_url)
    if isinstance(input_type, InputType.LNURL_AUTH):
        request_data = input_type.data
        print(f"Domain: {request_data.domain}")
        print(f"Action: {request_data.action}")

        # Show domain to user and ask for confirmation
        # This is important for security
    # ANCHOR_END: parse-lnurl-auth


def authenticate(sdk: BreezSdk, request_data: LnurlAuthRequestDetails):
    # ANCHOR: lnurl-auth
    # Perform LNURL authentication
    result = sdk.lnurl_auth(request_data)

    if isinstance(result, LnurlCallbackStatus.OK):
        print("Authentication successful")
    elif isinstance(result, LnurlCallbackStatus.ERROR_STATUS):
        print(f"Authentication failed: {result.data.reason}")
    # ANCHOR_END: lnurl-auth
