import logging
from breez_sdk_spark import BreezSdk, InputType, default_config, ExternalInputParser, Network


async def parse_input(sdk: BreezSdk):
    # ANCHOR: parse-inputs
    input_str = "an input to be parsed..."

    try:
        parsed_input = await sdk.parse(input=input_str)
        if isinstance(parsed_input, InputType.BITCOIN_ADDRESS):
            details = parsed_input[0]
            logging.debug(f"Input is Bitcoin address {details.address}")
        elif isinstance(parsed_input, InputType.BOLT11_INVOICE):
            details = parsed_input[0]
            amount = "unknown"
            if details.amount_msat:
                amount = str(details.amount_msat)
            logging.debug(f"Input is BOLT11 invoice for {amount} msats")
        elif isinstance(parsed_input, InputType.LNURL_PAY):
            details = parsed_input[0]
            logging.debug(
                f"Input is LNURL-Pay/Lightning address accepting "
                f"min/max {details.min_sendable}/{details.max_sendable} msats"
            )
        elif isinstance(parsed_input, InputType.LNURL_WITHDRAW):
            details = parsed_input[0]
            logging.debug(
                f"Input is LNURL-Withdraw for min/max "
                f"{details.min_withdrawable}/{details.max_withdrawable} msats"
            )
        # Other input types are available
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: parse-inputs

async def set_external_input_parsers():
    # ANCHOR: set-external-input-parsers
    # Create the default config
    config = default_config(network=Network.MAINNET)
    config.api_key = "<breez api key>"

    # Configure external parsers
    config.external_input_parsers = [
        ExternalInputParser(
            provider_id="provider_a",
            input_regex="^provider_a",
            parser_url="https://parser-domain.com/parser?input=<input>"
        ),
        ExternalInputParser(
            provider_id="provider_b",
            input_regex="^provider_b",
            parser_url="https://parser-domain.com/parser?input=<input>"
        )
    ]
    # ANCHOR_END: set-external-input-parsers
