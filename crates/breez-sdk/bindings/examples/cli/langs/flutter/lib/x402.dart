import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:args/args.dart';
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

/// x402 subcommand names (used for tab completion).
const x402CommandNames = ['x402 fetch'];

const _paymentRequiredHeader = 'payment-required';
const _paymentSignatureHeader = 'payment-signature';
const _paymentResponseHeader = 'payment-response';

/// Times to pay the same challenge again while its payment is pending. Fetching
/// again instead would bring a new challenge with a new invoice to pay.
const _pendingRetries = 3;

/// Times to send the paid request again while the server answers that the
/// payment may still settle.
const _settleRetries = 5;
const _settleRetryDelay = Duration(seconds: 2);

typedef X402Handler = Future<void> Function(BreezSdk sdk, List<String> args);

class _X402Entry {
  final String description;
  final X402Handler handler;
  const _X402Entry(this.description, this.handler);
}

Map<String, _X402Entry>? _registry;

Map<String, _X402Entry> _getRegistry() {
  return _registry ??= {
    'fetch': _X402Entry(
      'Fetch a URL, paying its x402 Lightning challenge if the server asks for one',
      _handleFetch,
    ),
  };
}

/// Dispatch an x402 subcommand given the args after 'x402'.
Future<void> dispatchX402Command(List<String> args, BreezSdk sdk) async {
  final registry = _getRegistry();

  if (args.isEmpty || args[0] == 'help' || args[0] == '--help') {
    print('\nx402 subcommands:\n');
    for (final entry in registry.entries.toList()..sort((a, b) => a.key.compareTo(b.key))) {
      print('  x402 ${entry.key.padRight(30)} ${entry.value.description}');
    }
    print('');
    return;
  }

  final subName = args[0];
  final subArgs = args.sublist(1);

  if (!registry.containsKey(subName)) {
    print("Unknown x402 subcommand: $subName. Use 'x402 help' for available commands.");
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

// --- fetch ---

Future<void> _handleFetch(BreezSdk sdk, List<String> args) async {
  const usage = 'x402 fetch <url> --max-amount-sats <sats> [options]';
  final parser =
      ArgParser(usageLineLength: 80)
        ..addOption(
          'max-amount-sats',
          mandatory: true,
          help: 'The most to spend in sats, routing fee included',
        )
        ..addOption('method', abbr: 'X', defaultsTo: 'GET', help: 'The HTTP method: GET or POST')
        ..addOption('body', abbr: 'd', help: 'The request body')
        ..addMultiOption(
          'header',
          abbr: 'H',
          splitCommas: false,
          help: 'A request header as "Name: value". Can be repeated',
        );
  final results = _parseArgs(parser, args, usage);
  if (results == null) return;
  if (results.rest.length != 1) {
    print('Usage: $usage');
    print(parser.usage);
    return;
  }
  final url = _asSent(results.rest.first);
  final maxAmountSats = BigInt.tryParse(results.option('max-amount-sats')!);
  if (maxAmountSats == null || maxAmountSats.isNegative) {
    print('Invalid --max-amount-sats: ${results.option('max-amount-sats')}');
    return;
  }
  final method = results.option('method')!;
  if (method != 'GET' && method != 'POST') {
    print('unsupported method $method: use GET or POST');
    return;
  }
  final body = results.option('body');
  if (method == 'GET' && body != null) {
    print('a GET request has no body: use -X POST to send one');
    return;
  }

  // The server hashes the headers it binds, so send every header
  // explicitly: anything the HTTP client added on its own would be
  // missing from the hash computed here.
  final headers = <String, String>{};
  for (final line in results.multiOption('header')) {
    final colon = line.indexOf(':');
    if (colon < 0) {
      print('header "$line" is not "Name: value"');
      return;
    }
    headers[line.substring(0, colon).trim().toLowerCase()] = line.substring(colon + 1).trim();
  }
  headers.putIfAbsent('accept', () => '*/*');

  final client = HttpClient()..userAgent = null;
  try {
    var response = await _send(client, method, url, headers, body);
    if (response.status != 402) {
      _printResponse(response);
      return;
    }
    final paymentRequired = response.header(_paymentRequiredHeader);
    if (paymentRequired == null) {
      print('402 response has no PAYMENT-REQUIRED header');
      return;
    }

    final x402 = sdk.getX402();
    final request = PayX402Request(
      paymentRequired: paymentRequired,
      request: X402Request.http(
        method: method,
        url: url,
        headers: [for (final e in headers.entries) X402Header(name: e.key, value: e.value)],
        body: body == null ? null : Uint8List.fromList(utf8.encode(body)),
      ),
      maxAmount: X402PaymentAmount.lightning(amountSats: maxAmountSats),
      conversionOptions: null,
      completionTimeoutSecs: null,
    );
    var paid = await x402.pay(request: request);
    for (var i = 0; i < _pendingRetries; i++) {
      if (paid.status == X402PaymentStatus.paid) break;
      print('Payment is pending, waiting for it to complete...');
      paid = await x402.pay(request: request);
    }
    // The payment signature proves the payment, so it isn't printed.
    print(
      'x402 payment ${paid.payment.id} is ${paid.status.name}: '
      '${paid.payment.amount} sats, fee ${paid.payment.fees} sats',
    );
    final signature = paid.paymentSignature;
    if (signature == null) {
      print('Payment is still pending. Check its status with list-payments.');
      return;
    }

    headers[_paymentSignatureHeader] = signature;
    response = await _send(client, method, url, headers, body);
    for (var i = 0; i < _settleRetries; i++) {
      if (!_settlementPending(response)) break;
      print('Payment is not settled yet, sending it again...');
      await Future<void>.delayed(_settleRetryDelay);
      response = await _send(client, method, url, headers, body);
    }
    _printResponse(response);
  } finally {
    client.close();
  }
}

class _Response {
  final int status;
  final HttpHeaders headers;
  final String body;
  const _Response(this.status, this.headers, this.body);

  String? header(String name) => headers[name]?.last;
}

Future<_Response> _send(
  HttpClient client,
  String method,
  String url,
  Map<String, String> headers,
  String? body,
) async {
  final request = await client.openUrl(method, Uri.parse(url));
  // Redirects aren't followed: the payment then commits to exactly the
  // request sent.
  request.followRedirects = false;
  // dart:io adds accept-encoding on its own.
  request.headers.removeAll(HttpHeaders.acceptEncodingHeader);
  for (final e in headers.entries) {
    request.headers.set(e.key, e.value);
  }
  if (method == 'POST' && body != null) {
    final bytes = utf8.encode(body);
    request.contentLength = bytes.length;
    request.add(bytes);
  }
  final response = await request.close();
  final bytes = await response.fold<BytesBuilder>(BytesBuilder(), (b, chunk) => b..add(chunk));
  if (const [301, 302, 303, 307, 308].contains(response.statusCode)) {
    final location = response.headers[HttpHeaders.locationHeader]?.last ?? '<no location>';
    throw HttpException(
      'endpoint returned a ${response.statusCode} redirect to $location, which this client does not follow',
    );
  }
  return _Response(
    response.statusCode,
    response.headers,
    utf8.decode(bytes.takeBytes(), allowMalformed: true),
  );
}

/// The URL as dart:io sends it: `http://host` goes out as `http://host/`, and
/// a fragment never goes out.
String _asSent(String url) {
  final uri = Uri.parse(url).removeFragment();
  return (uri.path.isEmpty ? uri.replace(path: '/') : uri).toString();
}

/// Whether the server answered that the payment may still settle: a 402 with
/// no new challenge and the core x402 specification's non-final reason.
bool _settlementPending(_Response response) {
  final settlement = response.header(_paymentResponseHeader);
  if (response.status != 402 || response.header(_paymentRequiredHeader) != null || settlement == null) {
    return false;
  }
  try {
    final json = jsonDecode(utf8.decode(base64.decode(settlement)));
    return json is Map && json['errorReason'] == 'settlement_pending';
  } on FormatException {
    return false;
  }
}

void _printResponse(_Response response) {
  print('HTTP ${response.status}');
  final settlement = response.header(_paymentResponseHeader);
  if (settlement != null) {
    try {
      print('PAYMENT-RESPONSE: ${utf8.decode(base64.decode(settlement), allowMalformed: true)}');
    } on FormatException {
      // Printed only when it decodes.
    }
  }
  print(response.body);
}
