import argparse
import asyncio
import time

from breez_sdk_spark import (
    CreateTokenAllowanceRequest,
    ListTokenAllowancesRequest,
    PaymentStatus,
    PullPaymentRequest,
    RevokeTokenAllowanceRequest,
    SdkError,
    TokenAllowanceLimit,
    TokenAllowanceRole,
)

from breez_cli.serialization import print_value

PULL_RETRY_INTERVAL = 5
PULL_RETRY_WINDOW = 10 * 60

TOKEN_ALLOWANCES_COMMAND_NAMES = [
    "token-allowances create",
    "token-allowances revoke",
    "token-allowances list",
]


def _parser(name, description=""):
    return argparse.ArgumentParser(prog=f"token-allowances {name}", description=description)


def _parse_limit(s):
    if s.lower() == "unlimited":
        return TokenAllowanceLimit.UNLIMITED
    try:
        amount = int(s)
    except ValueError:
        raise argparse.ArgumentTypeError(f"Invalid limit '{s}': expected an amount or 'unlimited'")
    return TokenAllowanceLimit.AMOUNT(amount=amount)


def _parse_role(s):
    mapping = {"owner": TokenAllowanceRole.OWNER, "spender": TokenAllowanceRole.SPENDER}
    result = mapping.get(s.lower())
    if result is None:
        raise argparse.ArgumentTypeError(f"Invalid role '{s}': expected 'owner' or 'spender'")
    return result


# --- create ---

def _build_create_parser():
    p = _parser("create", "Grant a spender an allowance over this wallet's tokens")
    p.add_argument("spender_address", help="Spark address of the spender")
    p.add_argument("token_identifier", help="Identifier of the token")
    p.add_argument("expiry_time", type=int, help="Expiry as a Unix timestamp in seconds")
    p.add_argument("--max-per-payment", required=True, type=_parse_limit,
                   help="Limit on one payment, in token base units, or 'unlimited'")
    p.add_argument("--max-total", required=True, type=_parse_limit,
                   help="Limit on all payments together, in token base units, or 'unlimited'")
    p.add_argument("-r", "--recipient", dest="allowed_recipients", action="append", default=None,
                   help="Spark address the spender may pay (repeatable, omit for any)")
    return p


async def _handle_create(sdk, args):
    response = await sdk.create_token_allowance(
        request=CreateTokenAllowanceRequest(
            spender_address=args.spender_address,
            token_identifier=args.token_identifier,
            expiry_time=args.expiry_time,
            max_per_payment=args.max_per_payment,
            max_total=args.max_total,
            allowed_recipients=args.allowed_recipients or [],
        )
    )
    print_value(response.allowance)


# --- revoke ---

def _build_revoke_parser():
    p = _parser("revoke", "Revoke a token allowance this wallet granted")
    p.add_argument("allowance_id", help="Identifier of the allowance")
    return p


async def _handle_revoke(sdk, args):
    await sdk.revoke_token_allowance(
        request=RevokeTokenAllowanceRequest(allowance_id=args.allowance_id)
    )
    print("Token allowance revoked")


# --- list ---

def _build_list_parser():
    p = _parser("list", "List token allowances this wallet granted (owner) or received (spender)")
    p.add_argument("role", nargs="?", default="owner", type=_parse_role,
                   help="owner or spender (default: owner)")
    p.add_argument("--counterparty-address", default=None,
                   help="Only allowances with this counterparty Spark address")
    p.add_argument("--token-identifier", default=None,
                   help="Only allowances for this token")
    p.add_argument("--include-inactive", type=lambda v: v.lower() in ("true", "1", "yes"),
                   default=None, help="Include revoked, expired and exhausted allowances")
    p.add_argument("--offset", type=int, default=None, help="Number of allowances to skip")
    p.add_argument("--limit", type=int, default=None, help="Maximum number of allowances to return")
    return p


async def _handle_list(sdk, args):
    response = await sdk.list_token_allowances(
        request=ListTokenAllowancesRequest(
            role=args.role,
            counterparty_address=args.counterparty_address,
            token_identifier=args.token_identifier,
            include_inactive=args.include_inactive,
            offset=args.offset,
            limit=args.limit,
        )
    )
    print_value(response.allowances)


# ---------------------------------------------------------------------------
# Pull-payment retry loop
# ---------------------------------------------------------------------------

def _may_still_settle(result):
    if isinstance(result, Exception):
        return isinstance(result, SdkError) and (
            hasattr(result, "network_error") or hasattr(result, "spark_error")
        )
    return result.status == PaymentStatus.PENDING


async def pull_until_settled(sdk, prepare_response):
    deadline = prepare_response.expiry_time + PULL_RETRY_WINDOW
    while True:
        try:
            response = await sdk.pull_payment(
                request=PullPaymentRequest(prepare_response=prepare_response)
            )
            now = int(time.time())
            if response.status != PaymentStatus.PENDING or now >= deadline:
                return response
            print(
                f"The pull {response.tx_hash} is pending, "
                f"checking again in {PULL_RETRY_INTERVAL}s"
            )
        except SdkError as e:
            now = int(time.time())
            if now >= deadline:
                raise
            is_retryable = "NetworkError" in type(e).__name__ or "SparkError" in type(e).__name__
            if not is_retryable:
                raise
            print(f"Sending the pull failed ({e}), trying again in {PULL_RETRY_INTERVAL}s")
        await asyncio.sleep(PULL_RETRY_INTERVAL)


# ---------------------------------------------------------------------------
# Registry and dispatch
# ---------------------------------------------------------------------------

def _build_token_allowances_registry():
    return {
        "create": (_build_create_parser(), _handle_create),
        "revoke": (_build_revoke_parser(), _handle_revoke),
        "list": (_build_list_parser(), _handle_list),
    }


_REGISTRY = None


def _get_registry():
    global _REGISTRY
    if _REGISTRY is None:
        _REGISTRY = _build_token_allowances_registry()
    return _REGISTRY


async def dispatch_token_allowances_command(args, sdk):
    registry = _get_registry()

    if not args or args[0] == "help":
        print("\nToken allowances subcommands:\n")
        for name, (parser, _) in sorted(registry.items()):
            desc = parser.description or ""
            print(f"  token-allowances {name:30s} {desc}")
        print()
        return

    sub_name = args[0]
    sub_args = args[1:]

    if sub_name not in registry:
        print(f"Unknown token-allowances subcommand: {sub_name}. "
              "Use 'token-allowances help' for available commands.")
        return

    parser, handler = registry[sub_name]
    try:
        parsed = parser.parse_args(sub_args)
    except SystemExit:
        return

    await handler(sdk, parsed)
