import 'package:args/args.dart';
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

import 'cli.dart';
import 'serialization.dart';

/// Token allowance subcommand names (used for tab completion).
const tokenAllowanceCommandNames = [
  'token-allowances create',
  'token-allowances revoke',
  'token-allowances list',
];

typedef TokenAllowanceHandler = Future<void> Function(BreezSdk sdk, List<String> args);

class _TokenAllowanceEntry {
  final String description;
  final TokenAllowanceHandler handler;
  const _TokenAllowanceEntry(this.description, this.handler);
}

Map<String, _TokenAllowanceEntry>? _registry;

Map<String, _TokenAllowanceEntry> _getRegistry() {
  return _registry ??= {
    'create': _TokenAllowanceEntry('Grant a spender an allowance over this wallet\'s tokens', _handleCreate),
    'revoke': _TokenAllowanceEntry('Revoke a token allowance this wallet granted', _handleRevoke),
    'list': _TokenAllowanceEntry(
      'List token allowances this wallet granted (owner) or received (spender)',
      _handleList,
    ),
  };
}

/// Dispatch a token-allowances subcommand given the args after 'token-allowances'.
Future<void> dispatchTokenAllowanceCommand(List<String> args, BreezSdk sdk) async {
  final registry = _getRegistry();

  if (args.isEmpty || args[0] == 'help' || args[0] == '--help') {
    print('\nToken allowance subcommands:\n');
    for (final entry in registry.entries.toList()..sort((a, b) => a.key.compareTo(b.key))) {
      print('  token-allowances ${entry.key.padRight(30)} ${entry.value.description}');
    }
    print('');
    return;
  }

  final subName = args[0];
  final subArgs = args.sublist(1);

  if (!registry.containsKey(subName)) {
    print(
      "Unknown token-allowances subcommand: $subName. Use 'token-allowances help' for available commands.",
    );
    return;
  }

  await registry[subName]!.handler(sdk, subArgs);
}

/// Parse [args] with [parser], returning `null` if the user asked for help
/// or if parsing fails (prints usage + error in that case).
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

TokenAllowanceLimit? _parseLimit(String s) {
  if (s.toLowerCase() == 'unlimited') {
    return const TokenAllowanceLimit.unlimited();
  }
  final amount = BigInt.tryParse(s);
  if (amount == null) return null;
  return TokenAllowanceLimit.amount(amount: amount);
}

TokenAllowanceRole? _parseRole(String s) {
  switch (s.toLowerCase()) {
    case 'owner':
      return TokenAllowanceRole.owner;
    case 'spender':
      return TokenAllowanceRole.spender;
    default:
      return null;
  }
}

// --- create ---

Future<void> _handleCreate(BreezSdk sdk, List<String> args) async {
  final parser =
      ArgParser(usageLineLength: 80)
        ..addOption(
          'max-per-payment',
          mandatory: true,
          help: 'Limit on one payment, in token base units, or "unlimited"',
        )
        ..addOption(
          'max-total',
          mandatory: true,
          help: 'Limit on all payments together, in token base units, or "unlimited"',
        )
        ..addMultiOption('recipient', abbr: 'r', help: 'Allowed recipient Spark address (repeatable)');
  final results = _parseArgs(
    parser,
    args,
    'token-allowances create <spender_address> <token_identifier> <expiry_time> '
    '--max-per-payment <limit> --max-total <limit> [-r <recipient>...]',
  );
  if (results == null) return;

  if (results.rest.length < 3) {
    print(
      'Usage: token-allowances create <spender_address> <token_identifier> <expiry_time> '
      '--max-per-payment <limit> --max-total <limit> [-r <recipient>...]',
    );
    return;
  }

  final spenderAddress = results.rest[0];
  final tokenIdentifier = results.rest[1];
  final expiryTime = BigInt.parse(results.rest[2]);

  final maxPerPaymentStr = results.option('max-per-payment')!;
  final maxPerPayment = _parseLimit(maxPerPaymentStr);
  if (maxPerPayment == null) {
    print("Invalid max-per-payment '$maxPerPaymentStr': expected an amount or 'unlimited'");
    return;
  }

  final maxTotalStr = results.option('max-total')!;
  final maxTotal = _parseLimit(maxTotalStr);
  if (maxTotal == null) {
    print("Invalid max-total '$maxTotalStr': expected an amount or 'unlimited'");
    return;
  }

  final allowedRecipients = results.multiOption('recipient');

  final response = await sdk.createTokenAllowance(
    request: CreateTokenAllowanceRequest(
      spenderAddress: spenderAddress,
      tokenIdentifier: tokenIdentifier,
      maxPerPayment: maxPerPayment,
      maxTotal: maxTotal,
      expiryTime: expiryTime,
      allowedRecipients: allowedRecipients,
    ),
  );
  printValue(response.allowance);
}

// --- revoke ---

Future<void> _handleRevoke(BreezSdk sdk, List<String> args) async {
  if (args.isEmpty || args.first == 'help' || args.first == '--help') {
    print('Usage: token-allowances revoke <allowance_id>');
    return;
  }
  final allowanceId = args.first;
  await sdk.revokeTokenAllowance(request: RevokeTokenAllowanceRequest(allowanceId: allowanceId));
  print('Token allowance revoked');
}

// --- list ---

Future<void> _handleList(BreezSdk sdk, List<String> args) async {
  final parser =
      ArgParser(usageLineLength: 80)
        ..addOption('counterparty-address', help: 'Only allowances with this counterparty Spark address')
        ..addOption('token-identifier', help: 'Only allowances for this token')
        ..addOption('include-inactive', help: 'Include revoked, expired and exhausted allowances')
        ..addOption('offset', help: 'Number of allowances to skip')
        ..addOption('limit', help: 'Maximum number of allowances to return');
  final results = _parseArgs(parser, args, 'token-allowances list [owner|spender] [options]');
  if (results == null) return;

  final roleStr = results.rest.isNotEmpty ? results.rest.first : 'owner';
  final role = _parseRole(roleStr);
  if (role == null) {
    print("Invalid role '$roleStr': expected 'owner' or 'spender'");
    return;
  }

  final counterpartyAddress = results.option('counterparty-address');
  final tokenIdentifier = results.option('token-identifier');
  final includeInactiveStr = results.option('include-inactive');
  final includeInactive =
      includeInactiveStr != null ? ['true', '1', 'yes'].contains(includeInactiveStr.toLowerCase()) : null;
  final offsetStr = results.option('offset');
  final offset = offsetStr != null ? int.parse(offsetStr) : null;
  final limitStr = results.option('limit');
  final limit = limitStr != null ? int.parse(limitStr) : null;

  final response = await sdk.listTokenAllowances(
    request: ListTokenAllowancesRequest(
      role: role,
      counterpartyAddress: counterpartyAddress,
      tokenIdentifier: tokenIdentifier,
      includeInactive: includeInactive,
      offset: offset,
      limit: limit,
    ),
  );
  printValue(response.allowances);
}

// --- pull-until-settled ---

const _pullRetryIntervalSecs = 5;
const _pullRetryWindowSecs = 600; // 10 minutes

Future<PullPaymentResponse> pullUntilSettled(BreezSdk sdk, PreparePullPaymentResponse prepareResponse) async {
  final deadlineSecs = prepareResponse.expiryTime + BigInt.from(_pullRetryWindowSecs);
  while (true) {
    PullPaymentResponse? response;
    Object? error;
    try {
      response = await sdk.pullPayment(request: PullPaymentRequest(prepareResponse: prepareResponse));
    } catch (e) {
      error = e;
    }

    final nowSecs = BigInt.from(DateTime.now().millisecondsSinceEpoch ~/ 1000);
    if (!_mayStillSettle(response, error) || nowSecs >= deadlineSecs) {
      if (error != null) throw error;
      return response!;
    }
    if (response != null) {
      print('The pull ${response.txHash} is pending, checking again in ${_pullRetryIntervalSecs}s');
    } else {
      print('Sending the pull failed ($error), trying again in ${_pullRetryIntervalSecs}s');
    }
    await Future<void>.delayed(const Duration(seconds: _pullRetryIntervalSecs));
  }
}

bool _mayStillSettle(PullPaymentResponse? response, Object? error) {
  if (response != null) {
    return response.status == PaymentStatus.pending;
  }
  if (error == null) return false;
  final msg = error.toString();
  return msg.contains('NetworkError') || msg.contains('SparkError');
}
