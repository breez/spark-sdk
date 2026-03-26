import argparse

from breez_sdk_spark import (
    RegisterWebhookRequest,
    UnregisterWebhookRequest,
    WebhookEventType,
)

from breez_cli.serialization import print_value

# Webhooks subcommand names (used for REPL completion)
WEBHOOKS_COMMAND_NAMES = [
    "webhooks register",
    "webhooks unregister",
    "webhooks list",
]


def _parser(name, description=""):
    return argparse.ArgumentParser(prog=f"webhooks {name}", description=description)


def _parse_event_type(s):
    mapping = {
        "lightning-receive": WebhookEventType.LIGHTNING_RECEIVE_FINISHED,
        "lightning-send": WebhookEventType.LIGHTNING_SEND_FINISHED,
        "coop-exit": WebhookEventType.COOP_EXIT_FINISHED,
        "static-deposit": WebhookEventType.STATIC_DEPOSIT_FINISHED,
    }
    result = mapping.get(s.lower())
    if result is None:
        raise ValueError(
            f"Unknown event type: {s}. Valid values: lightning-receive, "
            "lightning-send, coop-exit, static-deposit"
        )
    return result


# --- register ---

def _build_register_parser():
    p = _parser("register", "Register a new webhook")
    p.add_argument("url", help="URL that will receive webhook notifications")
    p.add_argument("secret", help="Secret for HMAC-SHA256 signature verification")
    p.add_argument("events", nargs="+",
                   help="Event types (lightning-receive, lightning-send, coop-exit, static-deposit)")
    return p

async def _handle_register(sdk, args):
    event_types = [_parse_event_type(e) for e in args.events]
    result = await sdk.register_webhook(
        request=RegisterWebhookRequest(
            url=args.url,
            secret=args.secret,
            event_types=event_types,
        )
    )
    print_value(result)


# --- unregister ---

def _build_unregister_parser():
    p = _parser("unregister", "Unregister a webhook")
    p.add_argument("webhook_id", help="ID of the webhook to unregister")
    return p

async def _handle_unregister(sdk, args):
    await sdk.unregister_webhook(
        request=UnregisterWebhookRequest(webhook_id=args.webhook_id)
    )
    print("Webhook unregistered successfully")


# --- list ---

def _build_list_parser():
    return _parser("list", "List all registered webhooks")

async def _handle_list(sdk, _args):
    result = await sdk.list_webhooks()
    print_value(result)


# ---------------------------------------------------------------------------
# Registry and dispatch
# ---------------------------------------------------------------------------

def _build_webhooks_registry():
    return {
        "register": (_build_register_parser(), _handle_register),
        "unregister": (_build_unregister_parser(), _handle_unregister),
        "list": (_build_list_parser(), _handle_list),
    }


_REGISTRY = None

def _get_registry():
    global _REGISTRY
    if _REGISTRY is None:
        _REGISTRY = _build_webhooks_registry()
    return _REGISTRY


async def dispatch_webhooks_command(args, sdk):
    """Dispatch a webhooks subcommand given the args after 'webhooks'."""
    registry = _get_registry()

    if not args or args[0] == "help":
        print("\nWebhooks subcommands:\n")
        for name, (parser, _) in sorted(registry.items()):
            desc = parser.description or ""
            print(f"  webhooks {name:30s} {desc}")
        print()
        return

    sub_name = args[0]
    sub_args = args[1:]

    if sub_name not in registry:
        print(f"Unknown webhooks subcommand: {sub_name}. Use 'webhooks help' for available commands.")
        return

    parser, handler = registry[sub_name]
    try:
        parsed = parser.parse_args(sub_args)
    except SystemExit:
        return

    await handler(sdk, parsed)
