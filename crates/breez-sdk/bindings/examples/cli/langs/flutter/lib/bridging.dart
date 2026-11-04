import 'package:args/args.dart';
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

import 'cli.dart';
import 'serialization.dart';

/// Bridging subcommand names (used for tab completion).
const bridgingCommandNames = [
  'bridging create',
  'bridging from-cash-app',
  'bridging to-cash-app',
  'bridging status',
];

typedef BridgingHandler = Future<void> Function(BreezSdk sdk, List<String> args);

class _BridgingEntry {
  final String description;
  final BridgingHandler handler;
  const _BridgingEntry(this.description, this.handler);
}

Map<String, _BridgingEntry>? _registry;

Map<String, _BridgingEntry> _getRegistry() {
  return _registry ??= {
    'create': _BridgingEntry(
      'Create a bridge that pays a recipient from another chain or over Lightning',
      _handleCreate,
    ),
    'from-cash-app': _BridgingEntry(
      'Bridge from Cash App to USDC/USDT on an external chain',
      _handleFromCashApp,
    ),
    'to-cash-app': _BridgingEntry(
      'Bridge USDC/USDT from an external chain to a Cash App user',
      _handleToCashApp,
    ),
    'status': _BridgingEntry("Read a bridge's status", _handleStatus),
  };
}

/// Dispatch a bridging subcommand given the args after 'bridging'.
Future<void> dispatchBridgingCommand(List<String> args, BreezSdk sdk) async {
  final registry = _getRegistry();

  if (args.isEmpty || args[0] == 'help' || args[0] == '--help') {
    print('\nBridging subcommands:\n');
    for (final entry in registry.entries.toList()..sort((a, b) => a.key.compareTo(b.key))) {
      print('  bridging ${entry.key.padRight(30)} ${entry.value.description}');
    }
    print('');
    return;
  }

  final subName = args[0];
  final subArgs = args.sublist(1);

  if (!registry.containsKey(subName)) {
    print("Unknown bridging subcommand: $subName. Use 'bridging help' for available commands.");
    return;
  }

  await registry[subName]!.handler(sdk, subArgs);
}

// ---------------------------------------------------------------------------
// Argument parser helpers
// ---------------------------------------------------------------------------

ArgResults? _parseArgs(ArgParser parser, List<String> args, String usage) {
  if (args.contains('help') || args.contains('--help') || args.contains('-h')) {
    print('Usage: $usage');
    print(parser.usage);
    return null;
  }
  try {
    return parser.parse(args);
  } on ArgParserException catch (e) {
    print('Usage: $usage');
    print(parser.usage);
    print('\nError: ${e.message}');
    return null;
  }
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

// --- bridging create ---

Future<void> _handleCreate(BreezSdk sdk, List<String> args) async {
  final parser =
      ArgParser(usageLineLength: 80)
        ..addOption('amount', mandatory: true, help: 'Amount in stablecoin base units')
        ..addOption('refund-address', help: "Payer's address on the source chain for refunds")
        ..addFlag('fees-included', defaultsTo: false, help: 'Deduct fees from the amount')
        ..addOption('max-slippage-bps', help: 'Maximum slippage in basis points');
  final results = _parseArgs(parser, args, 'bridging create <recipient> --amount <amount> [options]');
  if (results == null) return;

  if (results.rest.isEmpty) {
    print('Usage: bridging create <recipient> --amount <amount> [options]');
    return;
  }
  final recipient = results.rest.first;
  final amount = BigInt.parse(results.option('amount')!);
  final refundAddress = results.option('refund-address');
  final feesIncluded = results.flag('fees-included');
  final maxSlippageBpsStr = results.option('max-slippage-bps');
  final maxSlippageBps = maxSlippageBpsStr != null ? int.parse(maxSlippageBpsStr) : null;

  final client = sdk.getBridgingClient();
  final routesResponse = await client.getRoutes(request: GetBridgeRoutesRequest(recipient: recipient));
  final route = _chooseBridgeRoute(routesResponse.routes);
  if (route == null) return;

  final feePolicy = feesIncluded ? FeePolicy.feesIncluded : null;

  final response = await client.createBridge(
    request: CreateBridgeRequest(
      recipient: recipient,
      route: route,
      amount: amount,
      refundAddress: refundAddress,
      feePolicy: feePolicy,
      maxSlippageBps: maxSlippageBps,
    ),
  );
  print('Ask the payer to pay:');
  print(response.paymentRequest);
  printValue(response);
  _printBridgeTracking(response);
}

// --- bridging from-cash-app ---

Future<void> _handleFromCashApp(BreezSdk sdk, List<String> args) async {
  final parser =
      ArgParser(usageLineLength: 80)
        ..addOption(
          'amount',
          mandatory: true,
          help: 'Amount in stablecoin base units (6 decimals for USDC/USDT)',
        )
        ..addFlag('fees-included', defaultsTo: false, help: 'Deduct fees from the amount')
        ..addOption('max-slippage-bps', help: 'Maximum slippage in basis points');
  final results = _parseArgs(parser, args, 'bridging from-cash-app <recipient> --amount <amount> [options]');
  if (results == null) return;

  if (results.rest.isEmpty) {
    print('Usage: bridging from-cash-app <recipient> --amount <amount> [options]');
    return;
  }
  final recipient = results.rest.first;
  final amount = BigInt.parse(results.option('amount')!);
  final feesIncluded = results.flag('fees-included');
  final maxSlippageBpsStr = results.option('max-slippage-bps');
  final maxSlippageBps = maxSlippageBpsStr != null ? int.parse(maxSlippageBpsStr) : null;

  final client = sdk.getBridgingClient();
  final routesResponse = await client.getRoutes(request: GetBridgeRoutesRequest(recipient: recipient));
  final lightningRoutes = routesResponse.routes.where((r) => r.source.chain == 'lightning').toList();
  final route = _chooseBridgeRoute(lightningRoutes);
  if (route == null) return;

  final feePolicy = feesIncluded ? FeePolicy.feesIncluded : null;

  final response = await client.bridgeFromCashApp(
    request: CreateBridgeRequest(
      recipient: recipient,
      route: route,
      amount: amount,
      refundAddress: null,
      feePolicy: feePolicy,
      maxSlippageBps: maxSlippageBps,
    ),
  );
  final info = response.info;
  print('Open this URL in a browser to complete the purchase:');
  print(response.paymentRequest);
  print(
    'Deposit ~${info.depositAmount} sats; recipient receives ~${info.expectedReceivedAmount} '
    '${info.destinationAsset}, service fee ${info.serviceFeeAmount} '
    '${_serviceFeeDenomination(info.serviceFeeAsset, info.serviceFeeAssetDecimals)}, '
    'expires ${info.expiresAt}',
  );
  _printBridgeTracking(response);
}

// --- bridging to-cash-app ---

Future<void> _handleToCashApp(BreezSdk sdk, List<String> args) async {
  final parser =
      ArgParser(usageLineLength: 80)
        ..addOption(
          'amount',
          mandatory: true,
          help: 'Amount in stablecoin base units (6 decimals for USDC/USDT)',
        )
        ..addOption('refund-address', mandatory: true, help: 'Payer address on the source chain for refunds')
        ..addFlag('fees-included', defaultsTo: false, help: 'Deduct fees from the amount')
        ..addOption('max-slippage-bps', help: 'Maximum slippage in basis points');
  final results = _parseArgs(
    parser,
    args,
    'bridging to-cash-app <recipient> --amount <amount> --refund-address <addr> [options]',
  );
  if (results == null) return;

  if (results.rest.isEmpty) {
    print('Usage: bridging to-cash-app <recipient> --amount <amount> --refund-address <addr> [options]');
    return;
  }
  final recipient = results.rest.first;
  final amount = BigInt.parse(results.option('amount')!);
  final refundAddress = results.option('refund-address')!;
  final feesIncluded = results.flag('fees-included');
  final maxSlippageBpsStr = results.option('max-slippage-bps');
  final maxSlippageBps = maxSlippageBpsStr != null ? int.parse(maxSlippageBpsStr) : null;

  final client = sdk.getBridgingClient();
  final routesResponse = await client.getRoutes(request: GetBridgeRoutesRequest(recipient: recipient));
  final route = _chooseBridgeRoute(routesResponse.routes);
  if (route == null) return;

  final feePolicy = feesIncluded ? FeePolicy.feesIncluded : null;

  final response = await client.bridgeToCashApp(
    request: CreateBridgeRequest(
      recipient: recipient,
      route: route,
      amount: amount,
      refundAddress: refundAddress,
      feePolicy: feePolicy,
      maxSlippageBps: maxSlippageBps,
    ),
  );
  final info = response.info;
  print('Ask the payer to pay:');
  print(response.paymentRequest);
  print(
    'Deposit ${info.depositAmount} ${route.source.asset} on ${route.source.chain} '
    'to ${info.depositAddress}. Recipient receives ~${info.expectedReceivedAmount} sats, '
    'service fee ${info.serviceFeeAmount} '
    '${_serviceFeeDenomination(info.serviceFeeAsset, info.serviceFeeAssetDecimals)}, '
    'expires at ${info.expiresAt}',
  );
  _printBridgeTracking(response);
}

// --- bridging status ---

Future<void> _handleStatus(BreezSdk sdk, List<String> args) async {
  final parser = ArgParser(usageLineLength: 80)
    ..addOption('status-token', help: 'The status token from bridging create');
  final results = _parseArgs(parser, args, 'bridging status <bridge_id> [--status-token <token>]');
  if (results == null) return;

  if (results.rest.isEmpty) {
    print('Usage: bridging status <bridge_id> [--status-token <token>]');
    return;
  }
  final bridgeId = results.rest.first;
  final statusToken = results.option('status-token');

  final client = sdk.getBridgingClient();
  final response = await client.getBridgeStatus(
    request: GetBridgeStatusRequest(bridgeId: bridgeId, statusToken: statusToken),
  );
  printValue(response);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

BridgeRoutePair? _chooseBridgeRoute(List<BridgeRoutePair> routes) {
  if (routes.isEmpty) {
    print('No bridge routes available');
    return null;
  }

  if (routes.length == 1) {
    final route = routes.first;
    print('Auto-selected route: ${_routeLabel(route)}');
    return route;
  }

  print('Available bridge routes:');
  for (var i = 0; i < routes.length; i++) {
    print('  ${i + 1}. ${_routeLabel(routes[i])}');
  }

  final line = prompt('Select route: ');
  final choice = int.tryParse(line.trim());
  if (choice == null || choice < 1 || choice > routes.length) {
    print('Invalid selection');
    return null;
  }
  return routes[choice - 1];
}

String _routeLabel(BridgeRoutePair route) {
  return '${_sideLabel(route.source)} -> ${_sideLabel(route.destination)} [${route.provider}]';
}

String _sideLabel(BridgeRoutePairSide side) {
  return '${side.asset} on ${side.chain}${_maybeTruncateAddress(side.contractAddress)}';
}

String _maybeTruncateAddress(String? addr) {
  if (addr == null) return '';
  if (addr.length > 12) {
    return ' (${addr.substring(0, 6)}...${addr.substring(addr.length - 6)})';
  }
  return ' ($addr)';
}

void _printBridgeTracking(CreateBridgeResponse response) {
  final token = response.statusToken != null ? ' --status-token ${response.statusToken}' : '';
  print('Track it with: bridging status ${response.bridgeId}$token');
}

String _serviceFeeDenomination(String? asset, int? decimals) {
  if (asset == null) return 'sats';
  if (decimals != null) return '$asset ($decimals decimals)';
  return asset;
}
