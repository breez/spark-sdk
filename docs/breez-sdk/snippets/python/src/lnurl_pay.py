import logging
from breez_sdk_spark import (
    BreezSdk,
    InputType,
    LnurlPayRequest,
    PrepareLnurlPayRequest,
    PrepareLnurlPayResponse,
)


async def prepare_pay(sdk: BreezSdk):
    # ANCHOR: prepare-lnurl-pay
    # Endpoint can also be of the form:
    # lnurlp://domain.com/lnurl-pay?key=val
    # lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43r
    #     vv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3k
    #     vdnxx5crxwpjvyunsephsz36jf
    lnurl_pay_url = "lightning@address.com"
    try:
        parsed_input = await sdk.parse(lnurl_pay_url)
        if isinstance(parsed_input, InputType.LIGHTNING_ADDRESS):
            details = parsed_input[0]
            amount_sats = 5_000
            optional_comment = "<comment>"
            pay_request = details.pay_request
            optional_validate_success_action_url = True

            request = PrepareLnurlPayRequest(
                amount_sats=amount_sats,
                pay_request=pay_request,
                comment=optional_comment,
                validate_success_action_url=optional_validate_success_action_url,
            )
            prepare_response = await sdk.prepare_lnurl_pay(request=request)

            # If the fees are acceptable, continue to create the LNURL Pay
            fee_sats = prepare_response.fee_sats
            logging.debug(f"Fees: {fee_sats} sats")
            return prepare_response
    except Exception as error:
        logging.error(error)
        raise


# ANCHOR_END: prepare-lnurl-pay


async def pay(sdk: BreezSdk, prepare_response: PrepareLnurlPayResponse):
    # ANCHOR: lnurl-pay
    try:
        response = await sdk.lnurl_pay(
            LnurlPayRequest(prepare_response=prepare_response)
        )
    except Exception as error:
        logging.error(error)
        raise


# ANCHOR_END: lnurl-pay
