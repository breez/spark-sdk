import logging
import os
import shlex
from pathlib import Path

import asyncclick as click
from prompt_toolkit import PromptSession
from prompt_toolkit.auto_suggest import AutoSuggestFromHistory
from prompt_toolkit.completion import WordCompleter
from prompt_toolkit.history import FileHistory

from breez_sdk_spark import (
    EventListener,
    KeySetConfig,
    KeySetType,
    Network,
    SdkBuilder,
    SdkEvent,
    Seed,
    StableBalanceConfig,
    default_config,
    default_postgres_storage_config,
    init_logging,
)

from breez_cli.commands import COMMAND_NAMES, build_command_registry
from breez_cli.contacts import CONTACTS_COMMAND_NAMES, dispatch_contacts_command
from breez_cli.issuer import ISSUER_COMMAND_NAMES, dispatch_issuer_command
from breez_cli.webhooks import WEBHOOKS_COMMAND_NAMES, dispatch_webhooks_command
from breez_cli.passkey import create_provider, resolve_passkey_seed
from breez_cli.persistence import CliPersistence
from breez_cli.serialization import serialize

logger = logging.getLogger(__name__)


class CliEventListener(EventListener):
    async def on_event(self, event: SdkEvent):
        try:
            logger.info("Event: %s", serialize(event))
        except Exception:
            logger.info("Event: %s", repr(event))


def expand_path(path: str) -> Path:
    if path.startswith("~/"):
        return Path.home() / path[2:]
    return Path(path)


@click.command()
@click.option("-d", "--data-dir", default="./.data", help="Path to the data directory")
@click.option(
    "--network",
    default="regtest",
    type=click.Choice(["regtest", "mainnet"], case_sensitive=False),
    help="Network to use",
)
@click.option("--account-number", type=int, default=None, help="Account number for the Spark signer")
@click.option("--postgres-connection-string", default=None, help="PostgreSQL connection string")
@click.option("--stable-balance-token-identifier", default=None, help="Stable balance token identifier")
@click.option("--stable-balance-threshold", type=int, default=None, help="Stable balance threshold in sats")
@click.option("--passkey", "passkey_provider", default=None, help="Use passkey with file, yubikey, or fido2 provider")
@click.option("--label", default=None, help="Label for seed derivation (requires --passkey)")
@click.option("--list-labels", is_flag=True, default=False, help="List and select from labels published to Nostr (requires --passkey)")
@click.option("--store-label", is_flag=True, default=False, help="Publish the label to Nostr (requires --passkey and --label)")
@click.option("--rpid", default=None, help="Relying party ID for FIDO2 provider (requires --passkey)")
async def main(data_dir, network, account_number, postgres_connection_string,
               stable_balance_token_identifier, stable_balance_threshold,
               passkey_provider, label, list_labels, store_label, rpid):
    """CLI client for Breez SDK with Spark."""
    data_dir = expand_path(data_dir)
    data_dir.mkdir(parents=True, exist_ok=True)

    # Validate passkey flag combinations
    if label and not passkey_provider:
        raise click.UsageError("--label requires --passkey")
    if list_labels and not passkey_provider:
        raise click.UsageError("--list-labels requires --passkey")
    if store_label and not passkey_provider:
        raise click.UsageError("--store-label requires --passkey")
    if store_label and not label:
        raise click.UsageError("--store-label requires --label")
    if rpid and not passkey_provider:
        raise click.UsageError("--rpid requires --passkey")
    if list_labels and (label or store_label):
        raise click.UsageError("--list-labels conflicts with --label and --store-label")

    init_logging(log_dir=str(data_dir), app_logger=None, log_filter=None)

    persistence = CliPersistence(data_dir)

    network_enum = Network.MAINNET if network == "mainnet" else Network.REGTEST
    breez_api_key = os.environ.get("BREEZ_API_KEY")
    config = default_config(network=network_enum)
    config.api_key = breez_api_key

    if stable_balance_token_identifier:
        config.stable_balance_config = StableBalanceConfig(
            token_identifier=stable_balance_token_identifier,
            threshold_sats=stable_balance_threshold,
            max_slippage_bps=None,
            reserved_sats=None,
        )

    if passkey_provider:
        provider = create_provider(passkey_provider, data_dir, rpid=rpid)
        seed = await resolve_passkey_seed(
            provider, breez_api_key, label, list_labels, store_label,
        )
    else:
        mnemonic = persistence.get_or_create_mnemonic()
        seed = Seed.MNEMONIC(mnemonic=mnemonic, passphrase=None)
    builder = SdkBuilder(config=config, seed=seed)

    if postgres_connection_string:
        pg_config = default_postgres_storage_config(connection_string=postgres_connection_string)
        await builder.with_postgres_backend(config=pg_config)
    else:
        await builder.with_default_storage(storage_dir=str(data_dir))

    if account_number is not None:
        key_set_config = KeySetConfig(
            key_set_type=KeySetType.DEFAULT,
            use_address_index=False,
            account_number=account_number,
        )
        await builder.with_key_set(config=key_set_config)

    sdk = await builder.build()
    listener = CliEventListener()
    await sdk.add_event_listener(listener=listener)
    token_issuer = sdk.get_token_issuer()

    await run_repl(sdk, token_issuer, network_enum, persistence)


async def run_repl(sdk, token_issuer, network, persistence):
    history_file = persistence.history_file()
    all_commands = sorted(set(COMMAND_NAMES + CONTACTS_COMMAND_NAMES + ISSUER_COMMAND_NAMES + WEBHOOKS_COMMAND_NAMES + ["exit", "quit", "help"]))
    session = PromptSession(
        history=FileHistory(history_file),
        auto_suggest=AutoSuggestFromHistory(),
        completer=WordCompleter(all_commands, ignore_case=True),
    )

    network_label = "mainnet" if network == Network.MAINNET else "regtest"
    prompt_str = f"breez-spark-cli [{network_label}]> "

    print("Breez SDK CLI Interactive Mode")
    print("Type 'help' for available commands or 'exit' to quit")

    registry = build_command_registry()

    while True:
        try:
            line = await session.prompt_async(prompt_str)
            line = line.strip()
            if not line:
                continue

            if line in ("exit", "quit"):
                break

            if line == "help":
                print_help(registry)
                continue

            try:
                args = shlex.split(line)
            except ValueError as e:
                print(f"Parse error: {e}")
                continue

            cmd_name = args[0]
            cmd_args = args[1:]

            if cmd_name == "contacts":
                await dispatch_contacts_command(cmd_args, sdk)
            elif cmd_name == "issuer":
                await dispatch_issuer_command(cmd_args, token_issuer)
            elif cmd_name == "webhooks":
                await dispatch_webhooks_command(cmd_args, sdk)
            elif cmd_name in registry:
                parser, handler = registry[cmd_name]
                try:
                    parsed = parser.parse_args(cmd_args)
                except SystemExit:
                    # argparse calls sys.exit on --help or error; catch it
                    continue
                await handler(sdk, token_issuer, session, parsed)
            else:
                print(f"Unknown command: {cmd_name}. Type 'help' for available commands.")

        except KeyboardInterrupt:
            print("\nCTRL-C")
            break
        except EOFError:
            print("\nCTRL-D")
            break
        except Exception as e:
            print(f"Error: {e}")

    try:
        await sdk.disconnect()
    except Exception as e:
        logger.error("Failed to gracefully stop SDK: %s", e)

    print("Goodbye!")


def print_help(registry):
    print("\nAvailable commands:\n")
    for name in sorted(registry.keys()):
        parser, _ = registry[name]
        desc = parser.description or ""
        print(f"  {name:40s} {desc}")
    print(f"\n  {'contacts <subcommand>':40s} Contacts commands (use 'contacts help' for details)")
    print(f"  {'issuer <subcommand>':40s} Token issuer commands (use 'issuer help' for details)")
    print(f"  {'webhooks <subcommand>':40s} Webhook commands (use 'webhooks help' for details)")
    print(f"  {'exit / quit':40s} Exit the CLI")
    print(f"  {'help':40s} Show this help message")
    print()
