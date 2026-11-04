import argparse

from breez_sdk_spark import (
    CreateBridgeRequest,
    FeePolicy,
    GetBridgeRoutesRequest,
    GetBridgeStatusRequest,
)

from breez_cli.serialization import print_value

BRIDGING_COMMAND_NAMES = [
    "bridging create",
    "bridging from-cash-app",
    "bridging to-cash-app",
    "bridging status",
]


def _parser(name, description=""):
    return argparse.ArgumentParser(prog=f"bridging {name}", description=description)


def _service_fee_denomination(asset, decimals):
    if asset is None:
        return "sats"
    if decimals is not None:
        return f"{asset} ({decimals} decimals)"
    return asset


def _maybe_truncate_address(addr):
    if addr is None:
        return ""
    if len(addr) > 12:
        return f" ({addr[:6]}...{addr[-6:]})"
    return f" ({addr})"


def _route_label(route):
    return (
        f"{_side_label(route.source)} -> "
        f"{_side_label(route.destination)} "
        f"[{route.provider}]"
    )


def _side_label(side):
    return (
        f"{side.asset} on {side.chain}"
        f"{_maybe_truncate_address(side.contract_address)}"
    )


async def _choose_bridge_route(session, routes):
    if not routes:
        print("No bridge routes available")
        return None

    if len(routes) == 1:
        route = routes[0]
        print(f"Auto-selected route: {_route_label(route)}")
        return route

    print("Available bridge routes:")
    for i, route in enumerate(routes, 1):
        print(f"  {i}. {_route_label(route)}")

    line = await session.prompt_async("Select route: ")
    try:
        choice = int(line.strip())
    except ValueError:
        print("Invalid selection")
        return None
    if choice < 1 or choice > len(routes):
        print("Selection out of range")
        return None
    return routes[choice - 1]


def _print_bridge_tracking(response):
    token = ""
    if response.status_token is not None:
        token = f" --status-token {response.status_token}"
    print(f"Track it with: bridging status {response.bridge_id}{token}")


# --- create ---

def _build_create_parser():
    p = _parser("create",
                "Create a bridge that pays a recipient from another chain or over Lightning")
    p.add_argument("recipient",
                   help="Who to pay: a Spark address, a Lightning address or Cash App $cashtag, "
                   "or an EVM/Solana/Tron address")
    p.add_argument("--amount", type=int, required=True,
                   help="Amount in stablecoin base units")
    p.add_argument("--refund-address", default=None,
                   help="The payer's address on the source chain, refunded if delivery fails")
    p.add_argument("--fees-included", action="store_true", default=False,
                   help="Deduct fees from the amount instead of adding them on top")
    p.add_argument("--max-slippage-bps", type=int, default=None,
                   help="Maximum slippage in basis points")
    return p


async def _handle_create(sdk, session, args):
    client = sdk.get_bridging_client()
    routes_response = await client.get_routes(
        request=GetBridgeRoutesRequest(recipient=args.recipient)
    )
    route = await _choose_bridge_route(session, routes_response.routes)
    if route is None:
        return

    fee_policy = FeePolicy.FEES_INCLUDED if args.fees_included else None
    response = await client.create_bridge(
        request=CreateBridgeRequest(
            recipient=args.recipient,
            route=route,
            amount=args.amount,
            refund_address=args.refund_address,
            fee_policy=fee_policy,
            max_slippage_bps=args.max_slippage_bps,
        )
    )
    print("Ask the payer to pay:")
    print(response.payment_request)
    print_value(response)
    _print_bridge_tracking(response)


# --- from-cash-app ---

def _build_from_cash_app_parser():
    p = _parser("from-cash-app",
                "Bridge from Cash App to USDC/USDT on an external chain")
    p.add_argument("recipient",
                   help="Recipient address on the destination chain (EVM/Solana/Tron)")
    p.add_argument("--amount", type=int, required=True,
                   help="Amount in the destination stablecoin's base units "
                   "(6 decimals for USDC/USDT)")
    p.add_argument("--fees-included", action="store_true", default=False,
                   help="Deduct fees from the amount instead of adding them on top")
    p.add_argument("--max-slippage-bps", type=int, default=None,
                   help="Maximum slippage in basis points")
    return p


async def _handle_from_cash_app(sdk, session, args):
    client = sdk.get_bridging_client()
    routes_response = await client.get_routes(
        request=GetBridgeRoutesRequest(recipient=args.recipient)
    )
    routes = [r for r in routes_response.routes if r.source.chain == "lightning"]
    route = await _choose_bridge_route(session, routes)
    if route is None:
        return

    fee_policy = FeePolicy.FEES_INCLUDED if args.fees_included else None
    response = await client.bridge_from_cash_app(
        request=CreateBridgeRequest(
            recipient=args.recipient,
            route=route,
            amount=args.amount,
            refund_address=None,
            fee_policy=fee_policy,
            max_slippage_bps=args.max_slippage_bps,
        )
    )
    info = response.info
    print("Open this URL in a browser to complete the purchase:")
    print(response.payment_request)
    service_fee_denom = _service_fee_denomination(
        info.service_fee_asset, info.service_fee_asset_decimals,
    )
    print(
        f"Deposit ~{info.deposit_amount} sats; "
        f"recipient receives ~{info.expected_received_amount} {info.destination_asset}, "
        f"service fee {info.service_fee_amount} {service_fee_denom}, "
        f"expires {info.expires_at}"
    )
    _print_bridge_tracking(response)


# --- to-cash-app ---

def _build_to_cash_app_parser():
    p = _parser("to-cash-app",
                "Bridge USDC/USDT from an external chain to a Cash App user")
    p.add_argument("recipient",
                   help="Cash App $cashtag or Cash App Lightning address")
    p.add_argument("--amount", type=int, required=True,
                   help="Amount in the source stablecoin's base units "
                   "(6 decimals for USDC/USDT)")
    p.add_argument("--refund-address", required=True,
                   help="The payer's address on the source chain, refunded if delivery fails")
    p.add_argument("--fees-included", action="store_true", default=False,
                   help="Deduct fees from the amount instead of adding them on top")
    p.add_argument("--max-slippage-bps", type=int, default=None,
                   help="Maximum slippage in basis points")
    return p


async def _handle_to_cash_app(sdk, session, args):
    client = sdk.get_bridging_client()
    routes_response = await client.get_routes(
        request=GetBridgeRoutesRequest(recipient=args.recipient)
    )
    route = await _choose_bridge_route(session, routes_response.routes)
    if route is None:
        return

    fee_policy = FeePolicy.FEES_INCLUDED if args.fees_included else None
    response = await client.bridge_to_cash_app(
        request=CreateBridgeRequest(
            recipient=args.recipient,
            route=route,
            amount=args.amount,
            refund_address=args.refund_address,
            fee_policy=fee_policy,
            max_slippage_bps=args.max_slippage_bps,
        )
    )
    info = response.info
    print("Ask the payer to pay:")
    print(response.payment_request)
    service_fee_denom = _service_fee_denomination(
        info.service_fee_asset, info.service_fee_asset_decimals,
    )
    print(
        f"Deposit {info.deposit_amount} {route.source.asset} on {route.source.chain} "
        f"to {info.deposit_address}. "
        f"Recipient receives ~{info.expected_received_amount} sats, "
        f"service fee {info.service_fee_amount} {service_fee_denom}, "
        f"expires at {info.expires_at}"
    )
    _print_bridge_tracking(response)


# --- status ---

def _build_status_parser():
    p = _parser("status", "Read a bridge's status")
    p.add_argument("bridge_id", help="The bridge id that create returned")
    p.add_argument("--status-token", default=None,
                   help="The status token that bridging create returned")
    return p


async def _handle_status(sdk, _session, args):
    client = sdk.get_bridging_client()
    response = await client.get_bridge_status(
        request=GetBridgeStatusRequest(
            bridge_id=args.bridge_id,
            status_token=args.status_token,
        )
    )
    print_value(response)


# ---------------------------------------------------------------------------
# Registry and dispatch
# ---------------------------------------------------------------------------

def _build_bridging_registry():
    return {
        "create": (_build_create_parser(), _handle_create),
        "from-cash-app": (_build_from_cash_app_parser(), _handle_from_cash_app),
        "to-cash-app": (_build_to_cash_app_parser(), _handle_to_cash_app),
        "status": (_build_status_parser(), _handle_status),
    }


_REGISTRY = None


def _get_registry():
    global _REGISTRY
    if _REGISTRY is None:
        _REGISTRY = _build_bridging_registry()
    return _REGISTRY


async def dispatch_bridging_command(args, sdk, session):
    """Dispatch a bridging subcommand given the args after 'bridging'."""
    registry = _get_registry()

    if not args or args[0] == "help":
        print("\nBridging subcommands:\n")
        for name, (parser, _) in sorted(registry.items()):
            desc = parser.description or ""
            print(f"  bridging {name:30s} {desc}")
        print()
        return

    sub_name = args[0]
    sub_args = args[1:]

    if sub_name not in registry:
        print(f"Unknown bridging subcommand: {sub_name}. Use 'bridging help' for available commands.")
        return

    parser, handler = registry[sub_name]
    try:
        parsed = parser.parse_args(sub_args)
    except SystemExit:
        return

    await handler(sdk, session, parsed)
