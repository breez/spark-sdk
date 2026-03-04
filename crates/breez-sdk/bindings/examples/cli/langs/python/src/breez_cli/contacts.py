import argparse

from breez_sdk_spark import (
    AddContactRequest,
    ListContactsRequest,
    UpdateContactRequest,
)

from breez_cli.serialization import print_value

# Contacts subcommand names (used for REPL completion)
CONTACTS_COMMAND_NAMES = [
    "contacts add",
    "contacts update",
    "contacts delete",
    "contacts list",
]


def _parser(name, description=""):
    return argparse.ArgumentParser(prog=f"contacts {name}", description=description)


# --- add ---

def _build_add_parser():
    p = _parser("add", "Add a new contact")
    p.add_argument("name", help="Name of the contact")
    p.add_argument("payment_identifier", help="Lightning address (user@domain)")
    return p

async def _handle_add(sdk, args):
    result = await sdk.add_contact(
        request=AddContactRequest(
            name=args.name,
            payment_identifier=args.payment_identifier,
        )
    )
    print_value(result)


# --- update ---

def _build_update_parser():
    p = _parser("update", "Update an existing contact")
    p.add_argument("id", help="ID of the contact to update")
    p.add_argument("name", help="New name for the contact")
    p.add_argument("payment_identifier", help="New Lightning address (user@domain)")
    return p

async def _handle_update(sdk, args):
    result = await sdk.update_contact(
        request=UpdateContactRequest(
            id=args.id,
            name=args.name,
            payment_identifier=args.payment_identifier,
        )
    )
    print_value(result)


# --- delete ---

def _build_delete_parser():
    p = _parser("delete", "Delete a contact")
    p.add_argument("id", help="ID of the contact to delete")
    return p

async def _handle_delete(sdk, args):
    await sdk.delete_contact(id=args.id)
    print("Contact deleted successfully")


# --- list ---

def _build_list_parser():
    p = _parser("list", "List contacts")
    p.add_argument("offset", nargs="?", type=int, default=None,
                   help="Number of contacts to skip")
    p.add_argument("limit", nargs="?", type=int, default=None,
                   help="Maximum number of contacts to return")
    return p

async def _handle_list(sdk, args):
    result = await sdk.list_contacts(
        request=ListContactsRequest(
            offset=args.offset,
            limit=args.limit,
        )
    )
    print_value(result)


# ---------------------------------------------------------------------------
# Registry and dispatch
# ---------------------------------------------------------------------------

def _build_contacts_registry():
    return {
        "add": (_build_add_parser(), _handle_add),
        "update": (_build_update_parser(), _handle_update),
        "delete": (_build_delete_parser(), _handle_delete),
        "list": (_build_list_parser(), _handle_list),
    }


_REGISTRY = None

def _get_registry():
    global _REGISTRY
    if _REGISTRY is None:
        _REGISTRY = _build_contacts_registry()
    return _REGISTRY


async def dispatch_contacts_command(args, sdk):
    """Dispatch a contacts subcommand given the args after 'contacts'."""
    registry = _get_registry()

    if not args or args[0] == "help":
        print("\nContacts subcommands:\n")
        for name, (parser, _) in sorted(registry.items()):
            desc = parser.description or ""
            print(f"  contacts {name:30s} {desc}")
        print()
        return

    sub_name = args[0]
    sub_args = args[1:]

    if sub_name not in registry:
        print(f"Unknown contacts subcommand: {sub_name}. Use 'contacts help' for available commands.")
        return

    parser, handler = registry[sub_name]
    try:
        parsed = parser.parse_args(sub_args)
    except SystemExit:
        return

    await handler(sdk, parsed)
