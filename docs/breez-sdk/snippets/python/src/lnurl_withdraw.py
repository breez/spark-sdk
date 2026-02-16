import logging
from breez_sdk_spark import (
    BreezClient,
    InputType,
    LnurlWithdrawRequest,
)


async def withdraw(client: BreezClient):
    # ANCHOR: lnurl-withdraw
    # Endpoint can also be of the form:
    # lnurlw://domain.com/lnurl-withdraw?key=val
    lnurl_withdraw_url = (
        "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4exctthd96xserjv9mn7um9wdekj"
        "mmw843xxwpexdnxzen9vgunsvfexq6rvdecx93rgdmyxcuxverrvcursenpxvukzv3c8"
        "qunsdecx33nzwpnvg6ryc3hv93nzvecxgcxgwp3h33lxk"
    )

    try:
        input_type = await client.parse(lnurl_withdraw_url)
        if isinstance(input_type, InputType.LNURL_WITHDRAW):
            # Amount to withdraw in sats between min/max withdrawable amounts
            amount_sats = 5_000
            withdraw_request = input_type[0]
            optional_completion_timeout_secs = 30

            request = LnurlWithdrawRequest(
                amount_sats=amount_sats,
                withdraw_request=withdraw_request,
                completion_timeout_secs=optional_completion_timeout_secs,
            )
            response = await client.lnurl().withdraw(request=request)


            payment = response.payment
            logging.debug(f"Payment: {payment}")
            return response
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: lnurl-withdraw
