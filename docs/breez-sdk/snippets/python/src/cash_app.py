# pylint: disable=duplicate-code
import logging
from breez_sdk_spark import (
    BridgingClient,
    CreateBridgeRequest,
    GetBridgeRoutesRequest,
)


async def bridge_from_cash_app(client: BridgingClient):
    # ANCHOR: bridge-from-cash-app
    try:
        # List the routes to the recipient's external-chain address
        # (EVM/Solana/Tron) and pick a stablecoin Cash App can fund over
        # Lightning, e.g. USDC on Base.
        recipient = "<recipient address>"
        routes_response = await client.get_routes(
            request=GetBridgeRoutesRequest(recipient=recipient)
        )
        route = next(
            (
                r
                for r in routes_response.routes
                if r.source.chain == "lightning"
                and r.destination.asset == "USDC"
                and r.destination.chain == "base"
            ),
            None,
        )
        if route is None:
            raise ValueError("No USDC route on Base")

        # Send $10 of USDC, funded by Cash App over Lightning. The amount is in
        # USDC base units (6 decimals), so 10_000_000 = 10 USDC, about $10.
        request = CreateBridgeRequest(
            recipient=recipient,
            route=route,
            amount=10_000_000,
            refund_address=None,
            fee_policy=None,
            max_slippage_bps=None,
        )
        response = await client.bridge_from_cash_app(request=request)

        # Open this Cash App link to pay. The recipient then receives the
        # stablecoin.
        info = response.info
        logging.debug(f"Open this URL in Cash App: {response.payment_request}")
        logging.debug(
            f"Recipient receives ~{info.expected_received_amount} "
            f"{info.destination_asset}"
        )
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: bridge-from-cash-app


async def bridge_to_cash_app(client: BridgingClient):
    # ANCHOR: bridge-to-cash-app
    try:
        # List the routes to the Cash App user and pick what the payer pays in,
        # e.g. USDC on Base.
        recipient = "$alice"
        routes_response = await client.get_routes(
            request=GetBridgeRoutesRequest(recipient=recipient)
        )
        route = next(
            (
                r
                for r in routes_response.routes
                if r.source.asset == "USDC" and r.source.chain == "base"
            ),
            None,
        )
        if route is None:
            raise ValueError("No route from USDC on Base")

        # Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
        # units (6 decimals), so 10_000_000 = 10 USDC, about $10. The deposit is
        # refunded to the payer's address if delivery fails.
        request = CreateBridgeRequest(
            recipient=recipient,
            route=route,
            amount=10_000_000,
            refund_address="<payer address>",
            fee_policy=None,
            max_slippage_bps=None,
        )
        response = await client.bridge_to_cash_app(request=request)

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
