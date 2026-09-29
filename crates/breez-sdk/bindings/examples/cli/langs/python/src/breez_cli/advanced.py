import argparse
import json
import os

from breez_sdk_spark import (
    CheckRecoverFundsRequest,
    CheckUnilateralExitRequest,
    CooperativeRecoveryError,
    CooperativeRecoveryFailure,
    CpfpFundingKind,
    CpfpInput,
    ExitLeafSelection,
    ExitTransactionStatus,
    ImportUnilateralExitStateRequest,
    PrepareRecoverFundsRequest,
    PrepareUnilateralExitRequest,
    RecoverFundsLeaf,
    RecoverFundsRequest,
    RecoverFundsResponse,
    RecoveryMethod,
    RecoveryTransaction,
    RecoveryTxKind,
    RecoveryVerdict,
    UnilateralExitRequest,
    UnilateralExitVerdict,
    single_key_cpfp_signer,
)

from breez_cli.serialization import print_value, serialize

# Advanced subcommand names (used for REPL completion)
ADVANCED_COMMAND_NAMES = [
    "advanced unilateral-exit",
    "advanced check-unilateral-exit",
    "advanced recover-funds",
    "advanced check-recover-funds",
    "advanced export-unilateral-exit-state",
    "advanced import-unilateral-exit-state",
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
    p.add_argument("--output-file", default=None,
                   help="File to write the signed exit to, for check-unilateral-exit to read back")
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
        return CpfpInput.P2WPKH(txid=txid, vout=vout, value_sats=value, pubkey=pubkey)
    return CpfpInput.P2TR(txid=txid, vout=vout, value_sats=value, pubkey=pubkey)


def _print_exit_transactions(response):
    print(
        f"Recoverable {response.recoverable_value_sat} sats, "
        f"total fee {response.total_fee_sat} sats "
        f"(cpfp {response.cpfp_fee_sat}, fanout {response.fanout_fee_sat}, "
        f"sweep {response.sweep_fee_sat}), "
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
        if isinstance(tx.status, ExitTransactionStatus.CONFIRMED):
            block_height = tx.status.block_height
            if block_height is not None:
                print(f"      (confirmed in block {block_height}, nothing to broadcast)")
            else:
                print("      (already confirmed, nothing to broadcast)")
            continue
        if isinstance(tx.status, ExitTransactionStatus.WAITING_FOR_DEPENDENCIES):
            print("      (waiting on the transactions it depends on)")
        elif isinstance(tx.status, ExitTransactionStatus.WAITING_FOR_TIMELOCK):
            spendable_at_height = tx.status.spendable_at_height
            if spendable_at_height is not None:
                print(f"      (waiting for its timelock, until block {spendable_at_height})")
            else:
                print("      (waiting for its timelock)")
        if tx.cpfp_tx_hex is not None:
            package = f"{tx.tx_hex},{tx.cpfp_tx_hex}"
        else:
            package = tx.tx_hex
        print(f"      Package: {package}")


def _build_check_unilateral_exit_parser():
    p = _parser(
        "check-unilateral-exit",
        "Read a signed exit back against the chain: which transactions confirmed, what is ready to broadcast, and whether the exit still holds.",
    )
    p.add_argument("--input-file", required=True,
                   help="File the exit was written to")
    p.add_argument("--output-file", default=None,
                   help="File to write the updated exit to. Defaults to --input-file.")
    return p


def _build_export_unilateral_exit_state_parser():
    p = _parser(
        "export-unilateral-exit-state",
        "Export the wallet's unilateral exit state to a file, for safekeeping outside the wallet's own storage.",
    )
    p.add_argument("--output-file", required=True,
                   help="File to write the exit state to")
    return p


def _build_import_unilateral_exit_state_parser():
    p = _parser(
        "import-unilateral-exit-state",
        "Import a unilateral exit state previously written by export-unilateral-exit-state, merging it into the wallet.",
    )
    p.add_argument("--input-file", required=True,
                   help="File the exit state was exported to")
    return p


def _read_exit(path):
    with open(path) as f:
        return json.loads(f.read())


def _write_exit(path, response):
    with open(path, "w") as f:
        f.write(serialize(response))
    print(f"Wrote the exit to {path}")


async def _handle_check_unilateral_exit(sdk, _session, args):
    exit_data = _read_exit(args.input_file)
    checked = await sdk.check_unilateral_exit(
        request=CheckUnilateralExitRequest(exit=exit_data)
    )
    print(f"Verdict: {checked.verdict}")
    if isinstance(checked.verdict, UnilateralExitVerdict.REDO):
        print("  (this exit cannot be finished, quote and build it again)")
    _print_exit_transactions(checked.exit)
    output_file = args.output_file if args.output_file else args.input_file
    _write_exit(output_file, checked.exit)


async def _handle_export_unilateral_exit_state(sdk, _session, args):
    exported = await sdk.export_unilateral_exit_state()
    with open(args.output_file, "w") as f:
        f.write(exported.exit_state)
    print(
        f"Wrote {len(exported.exit_state)} bytes to {args.output_file}"
    )


async def _handle_import_unilateral_exit_state(sdk, _session, args):
    with open(args.input_file) as f:
        exit_state = f.read()
    imported = await sdk.import_unilateral_exit_state(
        request=ImportUnilateralExitStateRequest(exit_state=exit_state)
    )
    print(
        f"Imported {imported.imported_leaves} leaf(s), "
        f"skipped {imported.skipped_foreign_leaves} leaf(s) from a different wallet "
        f"and {imported.skipped_conflicting_leaves} that disagree with what this wallet holds, "
        f"left out the exit data of {imported.skipped_chains} leaf(s)"
    )


async def _handle_unilateral_exit(sdk, session, args):
    leaf_ids = args.leaf_ids or []
    if leaf_ids:
        selection = ExitLeafSelection.SPECIFIC(leaf_ids=leaf_ids)
    else:
        selection = ExitLeafSelection.ALL()

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
    if args.output_file:
        _write_exit(args.output_file, response)


# --- recover-funds ---

def _build_recover_funds_parser():
    p = _parser(
        "recover-funds",
        "Recover the funds that left the balance, or with --all every leaf. Quotes the recovery "
        "(which leaves, how each is recovered, the fees, how much to fund), asks for funding "
        "UTXOs and their key when a unilateral exit needs them, and signs it once you confirm. "
        "A cooperative recovery needs the operators online.",
    )
    p.add_argument("--fee-rate", type=int, required=True,
                   help="Target fee rate in sat/vByte")
    p.add_argument("--funding-kind", default="p2tr", choices=["p2wpkh", "p2tr"],
                   help="Funding UTXO kind (default: p2tr)")
    p.add_argument("--destination", required=True,
                   help="Destination address for the recovered funds")
    selection = p.add_mutually_exclusive_group()
    selection.add_argument("--all", action="store_true", default=False,
                           help="Recover every leaf worth it, including the ones still in the balance. "
                                "Only for when the operators are unreachable or refuse to serve the wallet.")
    selection.add_argument("--leaf", dest="leaf_ids", action="append", default=None,
                           help="Leaf id to recover (repeatable). Omit to recover the leaves that left the balance.")
    p.add_argument("--output-file", default=None,
                   help="File to write the signed recovery to, for check-recover-funds to read back")
    return p


def _build_check_recover_funds_parser():
    p = _parser(
        "check-recover-funds",
        "Read a recovery written by recover-funds back against the chain: which of its "
        "transactions confirmed, which are ready to broadcast now, and whether it can still finish.",
    )
    p.add_argument("--input-file", required=True,
                   help="File the recovery was written to")
    p.add_argument("--output-file", default=None,
                   help="File to write the updated recovery to. Defaults to --input-file.")
    return p


def _recovery_selection(all_leaves, leaf_ids):
    if all_leaves:
        return ExitLeafSelection.ALL()
    if not leaf_ids:
        return ExitLeafSelection.RECOVERABLE_ONLY()
    return ExitLeafSelection.SPECIFIC(leaf_ids=leaf_ids)


def _print_quote(prepared):
    print_value(prepared)
    cooperative = sum(1 for leaf in prepared.leaves if leaf.method == RecoveryMethod.COOPERATIVE)
    print(
        f"{len(prepared.leaves)} leaf(s), {cooperative} cooperative and "
        f"{len(prepared.leaves) - cooperative} unilateral: "
        f"recovering {prepared.recoverable_value_sats} sats for {prepared.total_fee_sats} sats in fees"
    )


def _cooperative_recovery_error_text(error):
    if isinstance(error, CooperativeRecoveryError.REPLACEMENT_FEE_TOO_LOW):
        return (
            "A recovery of this output is already on the network: replacing it takes at least "
            f"{error.required_fee_sats} sats or {error.required_fee_rate_sat_per_vbyte} sats/vbyte"
        )
    if isinstance(error, CooperativeRecoveryError.OPERATORS_UNAVAILABLE):
        return f"Operators unavailable: {error.message}"
    return f"Generic error: {error.message}"


def _print_recovery(response):
    print(
        f"Recoverable {response.recoverable_value_sats} sats, "
        f"total fee {response.total_fee_sats} sats "
        f"(cooperative {response.cooperative_fee_sats}, cpfp {response.cpfp_fee_sats}, "
        f"fanout {response.fanout_fee_sats}, sweep {response.sweep_fee_sats}), "
        f"{len(response.transactions)} transaction(s):"
    )
    for i, tx in enumerate(response.transactions):
        after = ""
        if tx.depends_on:
            after = f", after {','.join(tx.depends_on)}"
        csv = ""
        if tx.csv_timelock_blocks is not None:
            csv = f", csv {tx.csv_timelock_blocks} blocks"
        node = ""
        if tx.node_id is not None:
            node = f" node={tx.node_id}"
        print(f"  [{i}] {tx.kind}{node} status={tx.status} txid={tx.txid}{after}{csv}")
        if isinstance(tx.status, ExitTransactionStatus.CONFIRMED):
            block_height = tx.status.block_height
            if block_height is not None:
                print(f"      (confirmed in block {block_height}, nothing to broadcast)")
            else:
                print("      (already confirmed, nothing to broadcast)")
            continue
        if isinstance(tx.status, ExitTransactionStatus.WAITING_FOR_DEPENDENCIES):
            print("      (waiting on the transactions it depends on)")
        elif isinstance(tx.status, ExitTransactionStatus.WAITING_FOR_TIMELOCK):
            spendable_at_height = tx.status.spendable_at_height
            if spendable_at_height is not None:
                print(f"      (waiting for its timelock, until block {spendable_at_height})")
            else:
                print("      (waiting for its timelock)")
        if tx.cpfp_tx_hex is not None:
            package = f"{tx.tx_hex},{tx.cpfp_tx_hex}"
        else:
            package = tx.tx_hex
        print(f"      Package: {package}")
    if response.failed:
        print(f"Not recovered, {len(response.failed)} leaf(s):")
    for failure in response.failed:
        print(
            f"  leaf {failure.leaf_id} "
            f"(output {failure.output_txid}:{failure.output_vout}): "
            f"{_cooperative_recovery_error_text(failure.error)}"
        )


# serialize() writes a tagged enum variant as its fields plus its lower-cased
# name under "type", and a flat enum member as {"name", "value"}.
def _decode_variant(enum_type, data):
    fields = dict(data)
    return getattr(enum_type, fields.pop("type").upper())(**fields)


def _decode_recovery(data):
    return RecoverFundsResponse(
        recoverable_value_sats=data["recoverable_value_sats"],
        total_fee_sats=data["total_fee_sats"],
        cooperative_fee_sats=data["cooperative_fee_sats"],
        cpfp_fee_sats=data["cpfp_fee_sats"],
        fanout_fee_sats=data["fanout_fee_sats"],
        sweep_fee_sats=data["sweep_fee_sats"],
        leaves=[
            RecoverFundsLeaf(
                leaf_id=leaf["leaf_id"],
                value_sats=leaf["value_sats"],
                method=RecoveryMethod[leaf["method"]["name"]],
            )
            for leaf in data["leaves"]
        ],
        failed=[
            CooperativeRecoveryFailure(
                leaf_id=failure["leaf_id"],
                output_txid=failure["output_txid"],
                output_vout=failure["output_vout"],
                error=_decode_variant(CooperativeRecoveryError, failure["error"]),
            )
            for failure in data["failed"]
        ],
        transactions=[
            RecoveryTransaction(
                kind=RecoveryTxKind[tx["kind"]["name"]],
                node_id=tx["node_id"],
                txid=tx["txid"],
                tx_hex=tx["tx_hex"],
                cpfp_tx_hex=tx["cpfp_tx_hex"],
                csv_timelock_blocks=tx["csv_timelock_blocks"],
                depends_on=tx["depends_on"],
                status=_decode_variant(ExitTransactionStatus, tx["status"]),
            )
            for tx in data["transactions"]
        ],
        funding_inputs=[_decode_variant(CpfpInput, i) for i in data["funding_inputs"]],
        fee_rate_sat_per_vbyte=data["fee_rate_sat_per_vbyte"],
        destination=data["destination"],
    )


def _read_recovery(path):
    with open(path) as f:
        return _decode_recovery(json.loads(f.read()))


def _write_recovery(path, response):
    temporary = f"{path}.tmp"
    with open(temporary, "w") as f:
        f.write(serialize(response))
    os.replace(temporary, path)
    print(f"Wrote the recovery to {path}")


def _redo_command(recovery):
    command = (
        f"recover-funds --fee-rate {recovery.fee_rate_sat_per_vbyte} "
        f"--destination {recovery.destination}"
    )
    for leaf in recovery.leaves:
        command += f" --leaf {leaf.leaf_id}"
    return command


async def _recover_funds(sdk, session, request, funding_kind, output_file):
    prepared = await sdk.prepare_recover_funds(request=request)
    if not prepared.leaves:
        print(
            "Nothing to recover: each selected leaf is finished, not worth recovering at this "
            "fee rate, or its funds were not found."
        )
        return
    _print_quote(prepared)
    if not output_file:
        print(
            "Without --output-file the recovery is only printed: "
            "check-recover-funds cannot read it back."
        )

    funding_inputs = []
    signer = None
    if prepared.funding is not None:
        utxo_line = await session.prompt_async(
            f"Funding UTXO(s) of at least {prepared.funding.single_utxo_sats} sats, "
            "as txid:vout:value:pubkey (space-separated; for P2TR the internal key; "
            "blank to skip the unilateral exit): "
        )
        if not utxo_line.strip():
            cooperative = [
                leaf.leaf_id for leaf in prepared.leaves
                if leaf.method == RecoveryMethod.COOPERATIVE
            ]
            if not cooperative:
                print("Nothing to recover without funding.")
                return
            print("Recovering only the cooperative leaves:")
            prepared = await sdk.prepare_recover_funds(
                request=PrepareRecoverFundsRequest(
                    fee_rate_sat_per_vbyte=request.fee_rate_sat_per_vbyte,
                    funding_kind=request.funding_kind,
                    destination=request.destination,
                    selection=ExitLeafSelection.SPECIFIC(leaf_ids=cooperative),
                )
            )
            _print_quote(prepared)
        else:
            funding_inputs = [_parse_cpfp_input(u, funding_kind) for u in utxo_line.split()]
            key_line = await session.prompt_async("Hex secret key for the funding UTXO(s): ")
            signer = single_key_cpfp_signer(secret_key_bytes=bytes.fromhex(key_line.strip()))

    answer = await session.prompt_async("Sign this recovery? (y/n): ", default="y")
    if answer.strip().lower() != "y":
        return
    response = await sdk.recover_funds(
        request=RecoverFundsRequest(prepared=prepared, funding_inputs=funding_inputs),
        signer=signer,
    )
    _print_recovery(response)
    if output_file:
        _write_recovery(output_file, response)
        print(
            "Next: broadcast the Ready packages. After new blocks, run check-recover-funds "
            f"--input-file {output_file} to see what is ready next."
        )
    else:
        print("Next: broadcast the Ready packages.")


async def _handle_recover_funds(sdk, session, args):
    request = PrepareRecoverFundsRequest(
        fee_rate_sat_per_vbyte=args.fee_rate,
        funding_kind=_parse_funding_kind(args.funding_kind),
        destination=args.destination,
        selection=_recovery_selection(args.all, args.leaf_ids or []),
    )
    await _recover_funds(sdk, session, request, args.funding_kind, args.output_file)


async def _handle_check_recover_funds(sdk, _session, args):
    recovery = _read_recovery(args.input_file)
    checked = await sdk.check_recover_funds(
        request=CheckRecoverFundsRequest(recovery=recovery)
    )
    print(f"Verdict: {checked.verdict}")
    if isinstance(checked.verdict, RecoveryVerdict.REDO):
        print(f"  (this recovery cannot finish: run {_redo_command(checked.recovery)})")
    _print_recovery(checked.recovery)
    output_file = args.output_file if args.output_file else args.input_file
    _write_recovery(output_file, checked.recovery)


# ---------------------------------------------------------------------------
# Registry and dispatch
# ---------------------------------------------------------------------------

def _build_advanced_registry():
    return {
        "unilateral-exit": (_build_unilateral_exit_parser(), _handle_unilateral_exit),
        "check-unilateral-exit": (_build_check_unilateral_exit_parser(), _handle_check_unilateral_exit),
        "recover-funds": (_build_recover_funds_parser(), _handle_recover_funds),
        "check-recover-funds": (_build_check_recover_funds_parser(), _handle_check_recover_funds),
        "export-unilateral-exit-state": (_build_export_unilateral_exit_state_parser(), _handle_export_unilateral_exit_state),
        "import-unilateral-exit-state": (_build_import_unilateral_exit_state_parser(), _handle_import_unilateral_exit_state),
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
