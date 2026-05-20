import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

import 'serialization.dart';

/// Stable balance subcommand names (used for tab completion).
const stableBalanceCommandNames = ['stable-balance get', 'stable-balance set', 'stable-balance unset'];

typedef StableBalanceHandler = Future<void> Function(BreezSdk sdk, List<String> args);

class _StableBalanceEntry {
  final String description;
  final StableBalanceHandler handler;
  const _StableBalanceEntry(this.description, this.handler);
}

Map<String, _StableBalanceEntry>? _registry;

Map<String, _StableBalanceEntry> _getRegistry() {
  return _registry ??= {
    'get': _StableBalanceEntry('Get the stable balance active label', _handleGet),
    'set': _StableBalanceEntry('Set the stable balance active label', _handleSet),
    'unset': _StableBalanceEntry('Unset stable balance', _handleUnset),
  };
}

/// Dispatch a stable-balance subcommand given the args after 'stable-balance'.
Future<void> dispatchStableBalanceCommand(List<String> args, BreezSdk sdk) async {
  final registry = _getRegistry();

  if (args.isEmpty || args[0] == 'help' || args[0] == '--help') {
    print('\nStable balance subcommands:\n');
    for (final entry in registry.entries.toList()..sort((a, b) => a.key.compareTo(b.key))) {
      print('  stable-balance ${entry.key.padRight(30)} ${entry.value.description}');
    }
    print('');
    return;
  }

  final subName = args[0];
  final subArgs = args.sublist(1);

  if (!registry.containsKey(subName)) {
    print("Unknown stable-balance subcommand: $subName. Use 'stable-balance help' for available commands.");
    return;
  }

  await registry[subName]!.handler(sdk, subArgs);
}

// --- get ---

Future<void> _handleGet(BreezSdk sdk, List<String> args) async {
  final settings = await sdk.getUserSettings();
  printValue(settings.stableBalanceActiveLabel);
}

// --- set ---

Future<void> _handleSet(BreezSdk sdk, List<String> args) async {
  if (args.isEmpty || args.first == 'help' || args.first == '--help') {
    print('Usage: stable-balance set <label>');
    return;
  }
  final label = args[0];
  await sdk.updateUserSettings(
    request: UpdateUserSettingsRequest(stableBalanceActiveLabel: StableBalanceActiveLabel_Set(label: label)),
  );
  final settings = await sdk.getUserSettings();
  printValue(settings);
}

// --- unset ---

Future<void> _handleUnset(BreezSdk sdk, List<String> args) async {
  await sdk.updateUserSettings(
    request: UpdateUserSettingsRequest(stableBalanceActiveLabel: StableBalanceActiveLabel_Unset()),
  );
  final settings = await sdk.getUserSettings();
  printValue(settings);
}
