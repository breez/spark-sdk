import 'package:args/args.dart';
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

import 'serialization.dart';

/// Issuer subcommand names (used for help text).
const issuerCommandNames = [
  'issuer token-balance',
  'issuer token-metadata',
  'issuer create-token',
  'issuer mint-token',
  'issuer burn-token',
  'issuer freeze-token',
  'issuer unfreeze-token',
];

typedef IssuerHandler =
    Future<void> Function(TokenIssuer tokenIssuer, List<String> args);

class _IssuerEntry {
  final String description;
  final IssuerHandler handler;
  const _IssuerEntry(this.description, this.handler);
}

Map<String, _IssuerEntry>? _registry;

Map<String, _IssuerEntry> _getRegistry() {
  return _registry ??= {
    'token-balance': _IssuerEntry(
      'Get issuer token balance',
      _handleTokenBalance,
    ),
    'token-metadata': _IssuerEntry(
      'Get issuer token metadata',
      _handleTokenMetadata,
    ),
    'create-token': _IssuerEntry(
      'Create a new issuer token',
      _handleCreateToken,
    ),
    'mint-token': _IssuerEntry(
      'Mint supply of the issuer token',
      _handleMintToken,
    ),
    'burn-token': _IssuerEntry(
      'Burn supply of the issuer token',
      _handleBurnToken,
    ),
    'freeze-token': _IssuerEntry(
      'Freeze tokens at an address',
      _handleFreezeToken,
    ),
    'unfreeze-token': _IssuerEntry(
      'Unfreeze tokens at an address',
      _handleUnfreezeToken,
    ),
  };
}

/// Dispatch an issuer subcommand given the args after 'issuer'.
Future<void> dispatchIssuerCommand(
  List<String> args,
  TokenIssuer tokenIssuer,
) async {
  final registry = _getRegistry();

  if (args.isEmpty || args[0] == 'help') {
    print('\nIssuer subcommands:\n');
    for (final entry
        in registry.entries.toList()..sort((a, b) => a.key.compareTo(b.key))) {
      print('  issuer ${entry.key.padRight(30)} ${entry.value.description}');
    }
    print('');
    return;
  }

  final subName = args[0];
  final subArgs = args.sublist(1);

  if (!registry.containsKey(subName)) {
    print(
      "Unknown issuer subcommand: $subName. Use 'issuer help' for available commands.",
    );
    return;
  }

  await registry[subName]!.handler(tokenIssuer, subArgs);
}

// --- token-balance ---

Future<void> _handleTokenBalance(
  TokenIssuer tokenIssuer,
  List<String> args,
) async {
  final result = await tokenIssuer.getIssuerTokenBalance();
  printValue(result);
}

// --- token-metadata ---

Future<void> _handleTokenMetadata(
  TokenIssuer tokenIssuer,
  List<String> args,
) async {
  final result = await tokenIssuer.getIssuerTokenMetadata();
  printValue(result);
}

// --- create-token ---

Future<void> _handleCreateToken(
  TokenIssuer tokenIssuer,
  List<String> args,
) async {
  final parser =
      ArgParser()..addFlag('is-freezable', abbr: 'f', defaultsTo: false);
  final results = parser.parse(args);

  if (results.rest.length < 4) {
    print(
      'Usage: issuer create-token <name> <ticker> <decimals> <max_supply> [-f]',
    );
    return;
  }
  final name = results.rest[0];
  final ticker = results.rest[1];
  final decimals = int.parse(results.rest[2]);
  final maxSupply = BigInt.parse(results.rest[3]);
  final isFreezable = results.flag('is-freezable');

  final result = await tokenIssuer.createIssuerToken(
    request: CreateIssuerTokenRequest(
      name: name,
      ticker: ticker,
      decimals: decimals,
      isFreezable: isFreezable,
      maxSupply: maxSupply,
    ),
  );
  printValue(result);
}

// --- mint-token ---

Future<void> _handleMintToken(
  TokenIssuer tokenIssuer,
  List<String> args,
) async {
  if (args.isEmpty) {
    print('Usage: issuer mint-token <amount>');
    return;
  }
  final amount = BigInt.parse(args[0]);
  final result = await tokenIssuer.mintIssuerToken(
    request: MintIssuerTokenRequest(amount: amount),
  );
  printValue(result);
}

// --- burn-token ---

Future<void> _handleBurnToken(
  TokenIssuer tokenIssuer,
  List<String> args,
) async {
  if (args.isEmpty) {
    print('Usage: issuer burn-token <amount>');
    return;
  }
  final amount = BigInt.parse(args[0]);
  final result = await tokenIssuer.burnIssuerToken(
    request: BurnIssuerTokenRequest(amount: amount),
  );
  printValue(result);
}

// --- freeze-token ---

Future<void> _handleFreezeToken(
  TokenIssuer tokenIssuer,
  List<String> args,
) async {
  if (args.isEmpty) {
    print('Usage: issuer freeze-token <address>');
    return;
  }
  final address = args[0];
  final result = await tokenIssuer.freezeIssuerToken(
    request: FreezeIssuerTokenRequest(address: address),
  );
  printValue(result);
}

// --- unfreeze-token ---

Future<void> _handleUnfreezeToken(
  TokenIssuer tokenIssuer,
  List<String> args,
) async {
  if (args.isEmpty) {
    print('Usage: issuer unfreeze-token <address>');
    return;
  }
  final address = args[0];
  final result = await tokenIssuer.unfreezeIssuerToken(
    request: UnfreezeIssuerTokenRequest(address: address),
  );
  printValue(result);
}
