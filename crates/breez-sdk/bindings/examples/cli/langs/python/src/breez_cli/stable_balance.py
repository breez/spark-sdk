import argparse

from breez_sdk_spark import (
    StableBalanceActiveLabel,
    UpdateUserSettingsRequest,
)

from breez_cli.serialization import print_value

# Stable balance subcommand names (used for REPL completion)
STABLE_BALANCE_COMMAND_NAMES = [
    "stable-balance get",
    "stable-balance set",
    "stable-balance unset",
]


def _parser(name, description=""):
    return argparse.ArgumentParser(prog=f"stable-balance {name}", description=description)


# --- get ---

def _build_get_parser():
    return _parser("get", "Get the stable balance active label")

async def _handle_get(sdk, _args):
    settings = await sdk.get_user_settings()
    print_value(settings.stable_balance_active_label)


# --- set ---

def _build_set_parser():
    p = _parser("set", "Set the stable balance active label")
    p.add_argument("label", help='The label to activate (e.g. "USDB")')
    return p

async def _handle_set(sdk, args):
    await sdk.update_user_settings(
        request=UpdateUserSettingsRequest(
            spark_private_mode_enabled=None,
            stable_balance_active_label=StableBalanceActiveLabel.SET(label=args.label),
        )
    )
    settings = await sdk.get_user_settings()
    print_value(settings)


# --- unset ---

def _build_unset_parser():
    return _parser("unset", "Unset stable balance")

async def _handle_unset(sdk, _args):
    await sdk.update_user_settings(
        request=UpdateUserSettingsRequest(
            spark_private_mode_enabled=None,
            stable_balance_active_label=StableBalanceActiveLabel.UNSET(),
        )
    )
    settings = await sdk.get_user_settings()
    print_value(settings)


# ---------------------------------------------------------------------------
# Registry and dispatch
# ---------------------------------------------------------------------------

def _build_stable_balance_registry():
    return {
        "get": (_build_get_parser(), _handle_get),
        "set": (_build_set_parser(), _handle_set),
        "unset": (_build_unset_parser(), _handle_unset),
    }


_REGISTRY = None

def _get_registry():
    global _REGISTRY
    if _REGISTRY is None:
        _REGISTRY = _build_stable_balance_registry()
    return _REGISTRY


async def dispatch_stable_balance_command(args, sdk):
    """Dispatch a stable-balance subcommand given the args after 'stable-balance'."""
    registry = _get_registry()

    if not args or args[0] == "help":
        print("\nStable balance subcommands:\n")
        for name, (parser, _) in sorted(registry.items()):
            desc = parser.description or ""
            print(f"  stable-balance {name:30s} {desc}")
        print()
        return

    sub_name = args[0]
    sub_args = args[1:]

    if sub_name not in registry:
        print(f"Unknown stable-balance subcommand: {sub_name}. Use 'stable-balance help' for available commands.")
        return

    parser, handler = registry[sub_name]
    try:
        parsed = parser.parse_args(sub_args)
    except SystemExit:
        return

    await handler(sdk, parsed)
