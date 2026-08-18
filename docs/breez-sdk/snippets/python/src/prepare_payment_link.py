# pylint: disable=duplicate-code
import logging
from breez_sdk_spark import (
    BreezSdk,
    PreparePaymentLinkRequest,
    CrossChainRouteFilter,
    InputType,
)


async def prepare_payment_link_via_cashapp(sdk: BreezSdk):
    # ANCHOR: prepare-payment-link-cashapp
    # Parse the recipient's external-chain address (EVM/Solana/Tron).
    input_str = "<recipient address>"
    try:
        parsed = await sdk.parse(input=input_str)
        if not isinstance(parsed, InputType.CROSS_CHAIN_ADDRESS):
            raise ValueError("Not a cross-chain address")
        address_details = parsed[0]

        # List the stablecoin destinations you can send to and pick one,
        # e.g. USDC on Base. Restrict to Lightning-fundable routes.
        routes = await sdk.get_cross_chain_routes(
            filter=CrossChainRouteFilter.PAYMENT_LINK(
                address_details=address_details,
            )
        )
        route = next(
            (r for r in routes if r.asset == "USDC" and r.chain == "base"),
            None,
        )
        if route is None:
            raise ValueError("No USDC route on Base")

        # Send $10 of USDC (amount in USD 6-decimal base units,
        # 10_000_000 = $10.00), funded by Cash App over Lightning.
        request = PreparePaymentLinkRequest(
            address=address_details.address,
            route=route,
            amount=10_000_000,
            fee_policy=None,
            max_slippage_bps=None,
        )
        response = await sdk.prepare_payment_link(request=request)
        logging.debug(f"Open this URL in Cash App: {response.url}")
        logging.debug(
            f"Recipient receives ~{response.estimated_out} {response.asset}"
        )
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: prepare-payment-link-cashapp
