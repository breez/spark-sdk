import logging
from breez_sdk_spark import InputType, parse


async def parse_input():
    # ANCHOR: parse-inputs
    input_str = "an input to be parsed..."

    try:
        parsed_input = await parse(input=input_str)
        details = parsed_input[0]
        if isinstance(parsed_input, InputType.BITCOIN_ADDRESS):
            logging.debug(f"Input is Bitcoin address {details.address}")
        elif isinstance(parsed_input, InputType.BOLT11_INVOICE):
            amount = "unknown"
            if details.amount_msat:
                amount = str(details.amount_msat)
            logging.debug(f"Input is BOLT11 invoice for {amount} msats")
        elif isinstance(parsed_input, InputType.LNURL_PAY):
            logging.debug(
                f"Input is LNURL-Pay/Lightning address accepting "
                f"min/max {details.min_sendable}/{details.max_sendable} msats"
            )
        elif isinstance(parsed_input, InputType.LNURL_WITHDRAW):
            logging.debug(
                f"Input is LNURL-Withdraw for min/max "
                f"{details.min_withdrawable}/{details.max_withdrawable} msats"
            )
        # Other input types are available
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: parse-inputs
