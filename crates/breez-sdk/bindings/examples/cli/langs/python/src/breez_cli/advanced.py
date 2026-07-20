import argparse

from breez_sdk_spark import (
    ConfirmationStatus,
    CpfpFundingKind,
    CpfpInput,
    ExitLeafSelection,
    PrepareUnilateralExitRequest,
    UnilateralExitRequest,
    single_key_cpfp_signer,
)

from breez_cli.serialization import print_value

# Advanced subcommand names (used for REPL completion)
ADVANCED_COMMAND_NAMES = [
    "advanced unilateral-exit",
]


def _parser(name, description=""):
    return argparse.ArgumentParser(prog=f"advanced {name}", description=description)


# --- unilateral-exit ---

def _build_unilateral_exit_parser():
    p = _parser(
        "unilateral-exit",
        "Build and sign a unilateral exit. Quotes first, then prompts for funding UTXOs and signing key.",
    )
    p.add_argument("--fee-rate", type=int, required=True,
                   help="Target fee rate in sat/vByte")
    p.add_argument("--funding-kind", default="p2tr", choices=["p2wpkh", "p2tr"],
                   help="Funding UTXO kind (default: p2tr)")
    p.add_argument("--destination", required=True,
                   help="Destination address for the swept funds")
    p.add_argument("--leaf", dest="leaf_ids", action="append", default=None,
                   help="Leaf id to exit (repeatable). Omit to auto-select every profitable leaf.")
    return p


def _parse_funding_kind(kind_str):
    if kind_str == "p2wpkh":
        return CpfpFundingKind.P2WPKH()
    return CpfpFundingKind.P2TR()


def _parse_cpfp_input(s, funding_kind_str):
    parts = s.split(":")
    if len(parts) != 4:
        raise ValueError(f"Invalid funding UTXO '{s}', expected txid:vout:value:pubkey")
    txid, vout, value, pubkey = parts
    vout = int(vout)
    value = int(value)
    if funding_kind_str == "p2wpkh":
        return CpfpInput.P2WPKH(txid=txid, vout=vout, value=value, pubkey=pubkey)
    return CpfpInput.P2TR(txid=txid, vout=vout, value=value, pubkey=pubkey)


def _print_exit_transactions(response):
    print(
        f"Recoverable {response.recoverable_value_sat} sats, "
        f"total fee {response.total_fee_sat} sats, "
        f"{len(response.transactions)} transaction(s):"
    )
    for i, tx in enumerate(response.transactions):
        after = ""
        if tx.depends_on:
            after = f", after {','.join(tx.depends_on)}"
        csv = ""
        if tx.csv_timelock_blocks is not None:
            csv = f", csv {tx.csv_timelock_blocks} blocks"
        print(f"  [{i}] {tx.kind} status={tx.status} txid={tx.txid}{after}{csv}")
        if tx.status == ConfirmationStatus.CONFIRMED:
            print("      (already confirmed, nothing to broadcast)")
            continue
        if tx.cpfp_tx_hex is not None:
            package = f"{tx.tx_hex},{tx.cpfp_tx_hex}"
        else:
            package = tx.tx_hex
        print(f"      Package: {package}")


async def _handle_unilateral_exit(sdk, session, args):
    leaf_ids = args.leaf_ids or []
    if leaf_ids:
        selection = ExitLeafSelection.SPECIFIC(leaf_ids=leaf_ids)
    else:
        selection = ExitLeafSelection.AUTO()

    prepared = await sdk.prepare_unilateral_exit(
        request=PrepareUnilateralExitRequest(
            fee_rate_sat_per_vbyte=args.fee_rate,
            funding_kind=_parse_funding_kind(args.funding_kind),
            destination=args.destination,
            selection=selection,
        )
    )
    print_value(prepared)

    if not prepared.leaves:
        print("No leaves to exit.")
        return

    utxo_line = await session.prompt_async(
        "Funding UTXO(s) as txid:vout:value:pubkey (space-separated, blank to stop): "
    )
    if not utxo_line.strip():
        print("No funding provided; showing the quote only.")
        return

    funding_inputs = []
    for u in utxo_line.split():
        funding_inputs.append(_parse_cpfp_input(u, args.funding_kind))

    key_line = await session.prompt_async("Hex secret key for the funding UTXO(s): ")
    secret_key_bytes = bytes.fromhex(key_line.strip())
    signer = single_key_cpfp_signer(secret_key_bytes=secret_key_bytes)

    response = await sdk.unilateral_exit(
        request=UnilateralExitRequest(
            prepared=prepared,
            funding_inputs=funding_inputs,
        ),
        signer=signer,
    )
    _print_exit_transactions(response)


# ---------------------------------------------------------------------------
# Registry and dispatch
# ---------------------------------------------------------------------------

def _build_advanced_registry():
    return {
        "unilateral-exit": (_build_unilateral_exit_parser(), _handle_unilateral_exit),
    }


_REGISTRY = None

def _get_registry():
    global _REGISTRY
    if _REGISTRY is None:
        _REGISTRY = _build_advanced_registry()
    return _REGISTRY


async def dispatch_advanced_command(args, sdk, session):
    """Dispatch an advanced subcommand given the args after 'advanced'."""
    registry = _get_registry()

    if not args or args[0] == "help":
        print("\nAdvanced subcommands (expert-only, misuse can strand or lose funds):\n")
        for name, (parser, _) in sorted(registry.items()):
            desc = parser.description or ""
            print(f"  advanced {name:30s} {desc}")
        print()
        return

    sub_name = args[0]
    sub_args = args[1:]

    if sub_name not in registry:
        print(f"Unknown advanced subcommand: {sub_name}. Use 'advanced help' for available commands.")
        return

    parser, handler = registry[sub_name]
    try:
        parsed = parser.parse_args(sub_args)
    except SystemExit:
        return

    await handler(sdk, session, parsed)
