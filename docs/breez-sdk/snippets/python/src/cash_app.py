# pylint: disable=duplicate-code
import logging
from breez_sdk_spark import (
    BreezSdk,
    BridgeFromCashAppRequest,
    BridgeToCashAppRequest,
    CrossChainRouteFilter,
    DeliveryMethod,
    InputType,
)


async def bridge_from_cash_app(sdk: BreezSdk):
    # ANCHOR: bridge-from-cash-app
    # Parse the recipient's external-chain address (EVM/Solana/Tron).
    input_str = "<recipient address>"
    try:
        parsed = await sdk.parse(input=input_str)
        if not isinstance(parsed, InputType.CROSS_CHAIN_ADDRESS):
            raise ValueError("Not a cross-chain address")
        address_details = parsed[0]

        # List the stablecoin destinations Cash App can fund over Lightning and
        # pick one, e.g. USDC on Base.
        routes = await sdk.get_cross_chain_routes(
            filter=CrossChainRouteFilter.SEND(
                address_details=address_details,
                delivery_method=DeliveryMethod.LIGHTNING,
            )
        )
        route = next(
            (r for r in routes if r.asset == "USDC" and r.chain == "base"),
            None,
        )
        if route is None:
            raise ValueError("No USDC route on Base")

        # Send $10 of USDC, funded by Cash App over Lightning. The amount is in
        # the route asset's base units (USDC, 6 decimals), so 10_000_000 =
        # 10 USDC, about $10.
        request = BridgeFromCashAppRequest(
            address=address_details.address,
            route=route,
            amount=10_000_000,
            fee_policy=None,
            max_slippage_bps=None,
        )
        response = await sdk.bridge_from_cash_app(request=request)

        # Open this Cash App URL to pay. The recipient then receives the
        # stablecoin.
        logging.debug(f"Open this URL in Cash App: {response.url}")
        logging.debug(
            f"Recipient receives ~{response.estimated_out} {response.asset}"
        )
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: bridge-from-cash-app


async def bridge_to_cash_app(sdk: BreezSdk):
    # ANCHOR: bridge-to-cash-app
    try:
        # List the stablecoin sources that can pay a Cash App user over
        # Lightning and pick one, e.g. USDC on Base.
        routes = await sdk.get_cross_chain_routes(
            filter=CrossChainRouteFilter.RECEIVE(
                contract_address=None,
                delivery_method=DeliveryMethod.LIGHTNING,
            )
        )
        route = next(
            (r for r in routes if r.asset == "USDC" and r.chain == "base"),
            None,
        )
        if route is None:
            raise ValueError("No USDC route on Base")

        # Pay $10 of USDC to the Cash App user $alice. The amount is in the
        # route asset's base units (USDC, 6 decimals), so 10_000_000 =
        # 10 USDC, about $10. The deposit is refunded to the payer's address
        # if delivery fails.
        request = BridgeToCashAppRequest(
            recipient="$alice",
            route=route,
            amount=10_000_000,
            fee_policy=None,
            refund_address="<payer address>",
            max_slippage_bps=None,
        )
        response = await sdk.bridge_to_cash_app(request=request)

        # Show the payer what to pay. The recipient then receives Bitcoin.
        info = response.info
        logging.debug(f"Pay with: {response.payment_request}")
        logging.debug(
            f"Deposit {info.deposit_amount} to {info.deposit_address}, "
            f"recipient receives ~{info.expected_received_amount} sats"
        )
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: bridge-to-cash-app
