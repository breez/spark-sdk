import argparse

from breez_sdk_spark import (
    BurnIssuerTokenRequest,
    CreateIssuerTokenRequest,
    FreezeIssuerTokenRequest,
    MintIssuerTokenRequest,
    UnfreezeIssuerTokenRequest,
)

from breez_cli.serialization import print_value

# Issuer subcommand names (used for REPL completion)
ISSUER_COMMAND_NAMES = [
    "issuer token-balance",
    "issuer token-metadata",
    "issuer create-token",
    "issuer mint-token",
    "issuer burn-token",
    "issuer freeze-token",
    "issuer unfreeze-token",
]


def _parser(name, description=""):
    return argparse.ArgumentParser(prog=f"issuer {name}", description=description)


# --- token-balance ---

def _build_token_balance_parser():
    return _parser("token-balance", "Get issuer token balance")

async def _handle_token_balance(token_issuer, _args):
    result = await token_issuer.get_issuer_token_balance()
    print_value(result)


# --- token-metadata ---

def _build_token_metadata_parser():
    return _parser("token-metadata", "Get issuer token metadata")

async def _handle_token_metadata(token_issuer, _args):
    result = await token_issuer.get_issuer_token_metadata()
    print_value(result)


# --- create-token ---

def _build_create_token_parser():
    p = _parser("create-token", "Create a new issuer token")
    p.add_argument("name", help="Name of the token")
    p.add_argument("ticker", help="Ticker symbol")
    p.add_argument("decimals", type=int, help="Number of decimal places")
    p.add_argument("-f", "--is-freezable", action="store_true", default=False,
                   help="Whether the token is freezable")
    p.add_argument("max_supply", type=int, help="Maximum supply")
    return p

async def _handle_create_token(token_issuer, args):
    result = await token_issuer.create_issuer_token(
        CreateIssuerTokenRequest(
            name=args.name,
            ticker=args.ticker,
            decimals=args.decimals,
            is_freezable=args.is_freezable,
            max_supply=args.max_supply,
        )
    )
    print_value(result)


# --- mint-token ---

def _build_mint_token_parser():
    p = _parser("mint-token", "Mint supply of the issuer token")
    p.add_argument("amount", type=int, help="Amount to mint")
    return p

async def _handle_mint_token(token_issuer, args):
    result = await token_issuer.mint_issuer_token(
        MintIssuerTokenRequest(amount=args.amount)
    )
    print_value(result)


# --- burn-token ---

def _build_burn_token_parser():
    p = _parser("burn-token", "Burn supply of the issuer token")
    p.add_argument("amount", type=int, help="Amount to burn")
    return p

async def _handle_burn_token(token_issuer, args):
    result = await token_issuer.burn_issuer_token(
        BurnIssuerTokenRequest(amount=args.amount)
    )
    print_value(result)


# --- freeze-token ---

def _build_freeze_token_parser():
    p = _parser("freeze-token", "Freeze tokens at an address")
    p.add_argument("address", help="Address holding the tokens to freeze")
    return p

async def _handle_freeze_token(token_issuer, args):
    result = await token_issuer.freeze_issuer_token(
        FreezeIssuerTokenRequest(address=args.address)
    )
    print_value(result)


# --- unfreeze-token ---

def _build_unfreeze_token_parser():
    p = _parser("unfreeze-token", "Unfreeze tokens at an address")
    p.add_argument("address", help="Address holding the tokens to unfreeze")
    return p

async def _handle_unfreeze_token(token_issuer, args):
    result = await token_issuer.unfreeze_issuer_token(
        UnfreezeIssuerTokenRequest(address=args.address)
    )
    print_value(result)


# ---------------------------------------------------------------------------
# Registry and dispatch
# ---------------------------------------------------------------------------

def _build_issuer_registry():
    return {
        "token-balance": (_build_token_balance_parser(), _handle_token_balance),
        "token-metadata": (_build_token_metadata_parser(), _handle_token_metadata),
        "create-token": (_build_create_token_parser(), _handle_create_token),
        "mint-token": (_build_mint_token_parser(), _handle_mint_token),
        "burn-token": (_build_burn_token_parser(), _handle_burn_token),
        "freeze-token": (_build_freeze_token_parser(), _handle_freeze_token),
        "unfreeze-token": (_build_unfreeze_token_parser(), _handle_unfreeze_token),
    }


_REGISTRY = None

def _get_registry():
    global _REGISTRY
    if _REGISTRY is None:
        _REGISTRY = _build_issuer_registry()
    return _REGISTRY


async def dispatch_issuer_command(args, token_issuer):
    """Dispatch an issuer subcommand given the args after 'issuer'."""
    registry = _get_registry()

    if not args or args[0] == "help":
        print("\nIssuer subcommands:\n")
        for name, (parser, _) in sorted(registry.items()):
            desc = parser.description or ""
            print(f"  issuer {name:30s} {desc}")
        print()
        return

    sub_name = args[0]
    sub_args = args[1:]

    if sub_name not in registry:
        print(f"Unknown issuer subcommand: {sub_name}. Use 'issuer help' for available commands.")
        return

    parser, handler = registry[sub_name]
    try:
        parsed = parser.parse_args(sub_args)
    except SystemExit:
        return

    await handler(token_issuer, parsed)
