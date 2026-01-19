import logging
from breez_sdk_spark import (
    BreezSdk,
    InputType,
    LnurlPayRequest,
    LnurlPayRequestDetails,
    PayAmount,
    PrepareLnurlPayRequest,
    PrepareLnurlPayResponse,
    ConversionOptions,
    ConversionType,
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
            # Optionally set to use token funds to pay via token conversion
            optional_max_slippage_bps = 50
            optional_completion_timeout_secs = 30
            optional_conversion_options = ConversionOptions(
                conversion_type=ConversionType.TO_BITCOIN(
                    from_token_identifier="<token identifier>"
                ),
                max_slippage_bps=optional_max_slippage_bps,
                completion_timeout_secs=optional_completion_timeout_secs,
            )

            request = PrepareLnurlPayRequest(
                pay_amount = PayAmount.BITCOIN(amount_sats=amount_sats),
                pay_request=pay_request,
                comment=optional_comment,
                validate_success_action_url=optional_validate_success_action_url,
                conversion_options=optional_conversion_options,
            )
            prepare_response = await sdk.prepare_lnurl_pay(request=request)

            # If the fees are acceptable, continue to create the LNURL Pay
            if prepare_response.conversion_estimate is not None:
                conversion_estimate = prepare_response.conversion_estimate
                logging.debug(
                    f"Estimated conversion amount: {conversion_estimate.amount} token base units"
                )
                logging.debug(
                    f"Estimated conversion fee: {conversion_estimate.fee} token base units"
                )

            logging.debug(f"Fees: {prepare_response.fee_sats} sats")
            return prepare_response
    except Exception as error:
        logging.error(error)
        raise


# ANCHOR_END: prepare-lnurl-pay


async def pay(sdk: BreezSdk, prepare_response: PrepareLnurlPayResponse):
    # ANCHOR: lnurl-pay
    try:
        optional_idempotency_key = "<idempotency key uuid>"
        response = await sdk.lnurl_pay(
            LnurlPayRequest(
                prepare_response=prepare_response,
                idempotency_key=optional_idempotency_key,
            )
        )
    except Exception as error:
        logging.error(error)
        raise


# ANCHOR_END: lnurl-pay


async def prepare_pay_drain(sdk: BreezSdk, pay_request: LnurlPayRequestDetails):
    # ANCHOR: prepare-lnurl-pay-drain
    optional_comment = "<comment>"
    optional_validate_success_action_url = True
    pay_amount = PayAmount.DRAIN()

    request = PrepareLnurlPayRequest(
        pay_amount=pay_amount,
        pay_request=pay_request,
        comment=optional_comment,
        validate_success_action_url=optional_validate_success_action_url,
        conversion_options=None,
    )
    prepare_response = await sdk.prepare_lnurl_pay(request=request)

    # If the fees are acceptable, continue to create the LNURL Pay
    fee_sats = prepare_response.fee_sats
    logging.debug(f"Fees: {fee_sats} sats")
    # ANCHOR_END: prepare-lnurl-pay-drain
