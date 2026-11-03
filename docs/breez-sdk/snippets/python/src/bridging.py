# pylint: disable=duplicate-code
import logging
from typing import Optional
from breez_sdk_spark import (
    BreezSdk,
    BridgeStatus,
    BridgingClient,
    CreateBridgeRequest,
    default_bridging_config,
    GetBridgeRoutesRequest,
    GetBridgeStatusRequest,
    new_bridging_client,
)


async def new_client() -> BridgingClient:
    # ANCHOR: new-bridging-client
    config = default_bridging_config(api_key="<your api key>")
    try:
        client = await new_bridging_client(config=config)
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: new-bridging-client
    return client


def client_from_sdk(sdk: BreezSdk) -> BridgingClient:
    # ANCHOR: get-bridging-client
    try:
        client = sdk.get_bridging_client()
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: get-bridging-client
    return client


async def create_bridge(client: BridgingClient):
    # ANCHOR: create-bridge
    try:
        # Who to pay: a Spark address, a Lightning address or an EVM/Solana/Tron
        # address.
        recipient = "<recipient>"

        # List the routes that pay them and pick what the payer pays in, e.g.
        # USDC on Base.
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

        # Ask for $10. The amount is in the recipient's stablecoin when they are
        # paid in one, the payer's otherwise (6 decimals for USDC). The deposit
        # is refunded to the payer's address if the recipient can't be paid.
        request = CreateBridgeRequest(
            recipient=recipient,
            route=route,
            amount=10_000_000,
            # Required to pay a Lightning address.
            refund_address="<payer address on Base>",
            fee_policy=None,
            max_slippage_bps=None,
        )
        response = await client.create_bridge(request=request)

        # Show the payer what to pay, and store the bridge id and status token to
        # look up the status later.
        logging.debug(f"Pay with: {response.payment_request}")
        logging.debug(f"Bridge id: {response.bridge_id}")
    except Exception as error:
        logging.error(error)
        raise
    # ANCHOR_END: create-bridge


async def get_bridge_status(
    client: BridgingClient, bridge_id: str, status_token: Optional[str]
):
    # ANCHOR: get-bridge-status
    try:
        response = await client.get_bridge_status(
            request=GetBridgeStatusRequest(
                bridge_id=bridge_id, status_token=status_token
            )
        )
    except Exception as error:
        logging.error(error)
        raise
    status = response.status
    if status == BridgeStatus.COMPLETED:
        logging.debug(f"Delivered {response.amount_out}")
    elif status in (BridgeStatus.REFUNDED, BridgeStatus.FAILED, BridgeStatus.EXPIRED):
        logging.debug(f"Not delivered: {status}")
    else:
        logging.debug(f"Not final yet: {status}")
    # ANCHOR_END: get-bridge-status
