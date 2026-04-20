import 'dart:math';
import 'dart:typed_data';

import 'package:args/args.dart';
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'package:crypto/crypto.dart';

import 'cli.dart';
import 'serialization.dart';

/// All top-level command names (used for help text).
const commandNames = [
  'get-info',
  'get-payment',
  'sync',
  'list-payments',
  'receive',
  'pay',
  'lnurl-pay',
  'lnurl-withdraw',
  'lnurl-auth',
  'claim-htlc-payment',
  'claim-deposit',
  'parse',
  'refund-deposit',
  'list-unclaimed-deposits',
  'buy-bitcoin',
  'check-lightning-address-available',
  'get-lightning-address',
  'register-lightning-address',
  'delete-lightning-address',
  'list-fiat-currencies',
  'list-fiat-rates',
  'recommended-fees',
  'get-tokens-metadata',
  'fetch-conversion-limits',
  'get-user-settings',
  'set-user-settings',
  'get-spark-status',
];

typedef CommandHandler = Future<void> Function(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args);

class CommandEntry {
  final String description;
  final CommandHandler handler;
  const CommandEntry(this.description, this.handler);
}

/// Build the command registry.
Map<String, CommandEntry> buildCommandRegistry() {
  return {
    'get-info': CommandEntry('Get balance information', _handleGetInfo),
    'get-payment': CommandEntry('Get the payment with the given ID', _handleGetPayment),
    'sync': CommandEntry('Sync wallet state', _handleSync),
    'list-payments': CommandEntry('List payments', _handleListPayments),
    'receive': CommandEntry('Receive a payment', _handleReceive),
    'pay': CommandEntry('Pay the given payment request', _handlePay),
    'lnurl-pay': CommandEntry('Pay using LNURL', _handleLnurlPay),
    'lnurl-withdraw': CommandEntry('Withdraw using LNURL', _handleLnurlWithdraw),
    'lnurl-auth': CommandEntry('Authenticate using LNURL', _handleLnurlAuth),
    'claim-htlc-payment': CommandEntry('Claim an HTLC payment', _handleClaimHtlcPayment),
    'claim-deposit': CommandEntry('Claim an on-chain deposit', _handleClaimDeposit),
    'parse': CommandEntry('Parse an input (invoice, address, LNURL)', _handleParse),
    'refund-deposit': CommandEntry('Refund an on-chain deposit', _handleRefundDeposit),
    'list-unclaimed-deposits': CommandEntry('List unclaimed on-chain deposits', _handleListUnclaimedDeposits),
    'buy-bitcoin': CommandEntry('Buy Bitcoin using an external provider', _handleBuyBitcoin),
    'check-lightning-address-available': CommandEntry(
      'Check if a lightning address username is available',
      _handleCheckLightningAddressAvailable,
    ),
    'get-lightning-address': CommandEntry('Get registered lightning address', _handleGetLightningAddress),
    'register-lightning-address': CommandEntry(
      'Register a lightning address',
      _handleRegisterLightningAddress,
    ),
    'delete-lightning-address': CommandEntry('Delete lightning address', _handleDeleteLightningAddress),
    'list-fiat-currencies': CommandEntry('List fiat currencies', _handleListFiatCurrencies),
    'list-fiat-rates': CommandEntry('List available fiat rates', _handleListFiatRates),
    'recommended-fees': CommandEntry('Get recommended BTC fees', _handleRecommendedFees),
    'get-tokens-metadata': CommandEntry('Get metadata for token(s)', _handleGetTokensMetadata),
    'fetch-conversion-limits': CommandEntry(
      'Fetch conversion limits for a token',
      _handleFetchConversionLimits,
    ),
    'get-user-settings': CommandEntry('Get user settings', _handleGetUserSettings),
    'set-user-settings': CommandEntry('Update user settings', _handleSetUserSettings),
    'get-spark-status': CommandEntry('Get Spark network service status', _handleGetSparkStatus),
  };
}

// ---------------------------------------------------------------------------
// Argument parser helpers
// ---------------------------------------------------------------------------

ArgParser _parser(String name) => ArgParser(usageLineLength: 80);

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

bool? _parseBool(String? value) {
  if (value == null) return null;
  return ['true', '1', 'yes'].contains(value.toLowerCase());
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

// --- get-info ---

Future<void> _handleGetInfo(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final parser = _parser('get-info')..addOption('ensure-synced', abbr: 'e');
  final results = _parseArgs(parser, args, 'get-info [options]');
  if (results == null) return;
  final ensureSynced = _parseBool(results.option('ensure-synced'));
  final result = await sdk.getInfo(request: GetInfoRequest(ensureSynced: ensureSynced));
  printValue(result);
}

// --- get-payment ---

Future<void> _handleGetPayment(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final parser = _parser('get-payment');
  if (args.isEmpty || args.contains('help') || args.contains('--help')) {
    print('Usage: get-payment <payment_id>');
    return;
  }
  parser.parse(args);
  final paymentId = args.last;
  final result = await sdk.getPayment(request: GetPaymentRequest(paymentId: paymentId));
  printValue(result);
}

// --- sync ---

Future<void> _handleSync(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final result = await sdk.syncWallet(request: SyncWalletRequest());
  printValue(result);
}

// --- list-payments ---

PaymentType? _parsePaymentType(String s) {
  switch (s.toLowerCase()) {
    case 'send':
      return PaymentType.send;
    case 'receive':
      return PaymentType.receive;
    default:
      return null;
  }
}

PaymentStatus? _parsePaymentStatus(String s) {
  switch (s.toLowerCase()) {
    case 'completed':
      return PaymentStatus.completed;
    case 'pending':
      return PaymentStatus.pending;
    case 'failed':
      return PaymentStatus.failed;
    default:
      return null;
  }
}

SparkHtlcStatus? _parseHtlcStatus(String s) {
  switch (s) {
    case 'WaitingForPreimage':
      return SparkHtlcStatus.waitingForPreimage;
    case 'PreimageShared':
      return SparkHtlcStatus.preimageShared;
    case 'Returned':
      return SparkHtlcStatus.returned;
    default:
      return null;
  }
}

TokenTransactionType? _parseTxType(String s) {
  switch (s.toLowerCase()) {
    case 'mint':
      return TokenTransactionType.mint;
    case 'burn':
      return TokenTransactionType.burn;
    case 'transfer':
      return TokenTransactionType.transfer;
    default:
      return null;
  }
}

Future<void> _handleListPayments(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final parser =
      _parser('list-payments')
        ..addMultiOption('type-filter', abbr: 't')
        ..addMultiOption('status-filter', abbr: 's')
        ..addOption('asset-filter', abbr: 'a')
        ..addMultiOption('spark-htlc-status-filter')
        ..addOption('tx-hash')
        ..addOption('tx-type')
        ..addOption('from-timestamp')
        ..addOption('to-timestamp')
        ..addOption('limit', abbr: 'l', defaultsTo: '10')
        ..addOption('offset', abbr: 'o', defaultsTo: '0')
        ..addOption('sort-ascending');
  final results = _parseArgs(parser, args, 'list-payments [options]');
  if (results == null) return;

  List<PaymentType>? typeFilter;
  final typeFilterValues = results.multiOption('type-filter');
  if (typeFilterValues.isNotEmpty) {
    typeFilter = typeFilterValues.map(_parsePaymentType).whereType<PaymentType>().toList();
  }

  List<PaymentStatus>? statusFilter;
  final statusFilterValues = results.multiOption('status-filter');
  if (statusFilterValues.isNotEmpty) {
    statusFilter = statusFilterValues.map(_parsePaymentStatus).whereType<PaymentStatus>().toList();
  }

  AssetFilter? assetFilter;
  final assetFilterStr = results.option('asset-filter');
  if (assetFilterStr != null) {
    if (assetFilterStr.toLowerCase() == 'bitcoin') {
      assetFilter = AssetFilter.bitcoin();
    } else {
      assetFilter = AssetFilter.token(tokenIdentifier: assetFilterStr);
    }
  }

  final paymentDetailsFilter = <PaymentDetailsFilter>[];
  final htlcStatusValues = results.multiOption('spark-htlc-status-filter');
  if (htlcStatusValues.isNotEmpty) {
    final statuses = htlcStatusValues.map(_parseHtlcStatus).whereType<SparkHtlcStatus>().toList();
    if (statuses.isNotEmpty) {
      paymentDetailsFilter.add(PaymentDetailsFilter.spark(htlcStatus: statuses));
    }
  }
  final txHash = results.option('tx-hash');
  if (txHash != null) {
    paymentDetailsFilter.add(PaymentDetailsFilter.token(txHash: txHash));
  }
  final txTypeStr = results.option('tx-type');
  if (txTypeStr != null) {
    final txType = _parseTxType(txTypeStr);
    if (txType != null) {
      paymentDetailsFilter.add(PaymentDetailsFilter.token(txType: txType));
    }
  }

  final fromTimestampStr = results.option('from-timestamp');
  final toTimestampStr = results.option('to-timestamp');

  final result = await sdk.listPayments(
    request: ListPaymentsRequest(
      limit: int.parse(results.option('limit')!),
      offset: int.parse(results.option('offset')!),
      typeFilter: typeFilter,
      statusFilter: statusFilter,
      assetFilter: assetFilter,
      paymentDetailsFilter: paymentDetailsFilter.isNotEmpty ? paymentDetailsFilter : null,
      fromTimestamp: fromTimestampStr != null ? BigInt.parse(fromTimestampStr) : null,
      toTimestamp: toTimestampStr != null ? BigInt.parse(toTimestampStr) : null,
      sortAscending: _parseBool(results.option('sort-ascending')),
    ),
  );
  printValue(result);
}

// --- receive ---

Future<void> _handleReceive(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final parser =
      _parser('receive')
        ..addOption('method', abbr: 'm', mandatory: true, help: 'sparkaddress, sparkinvoice, bitcoin, bolt11')
        ..addOption('description', abbr: 'd')
        ..addOption('amount', abbr: 'a')
        ..addOption('token-identifier', abbr: 't')
        ..addOption('expiry-secs', abbr: 'e')
        ..addOption('sender-public-key', abbr: 's')
        ..addFlag('hodl', defaultsTo: false)
        ..addFlag('new-address', defaultsTo: false, help: 'Get a new bitcoin deposit address');
  final results = _parseArgs(parser, args, 'receive -m <method> [options]');
  if (results == null) return;

  final method = results.option('method')!.toLowerCase();
  final description = results.option('description');
  final amountStr = results.option('amount');
  final amount = amountStr != null ? BigInt.parse(amountStr) : null;
  final tokenIdentifier = results.option('token-identifier');
  final expirySecsStr = results.option('expiry-secs');
  final expirySecs = expirySecsStr != null ? int.parse(expirySecsStr) : null;
  final senderPublicKey = results.option('sender-public-key');
  final hodl = results.flag('hodl');
  final newAddress = results.flag('new-address');

  ReceivePaymentMethod paymentMethod;
  switch (method) {
    case 'sparkaddress':
      paymentMethod = ReceivePaymentMethod.sparkAddress();
    case 'sparkinvoice':
      BigInt? expiryTime;
      if (expirySecs != null) {
        expiryTime = BigInt.from(DateTime.now().millisecondsSinceEpoch ~/ 1000 + expirySecs);
      }
      paymentMethod = ReceivePaymentMethod.sparkInvoice(
        amount: amount,
        tokenIdentifier: tokenIdentifier,
        expiryTime: expiryTime,
        description: description,
        senderPublicKey: senderPublicKey,
      );
    case 'bitcoin':
      paymentMethod = ReceivePaymentMethod.bitcoinAddress(newAddress: newAddress);
    case 'bolt11':
      String? paymentHash;
      if (hodl) {
        final random = Random.secure();
        final preimageBytes = Uint8List(32);
        for (var i = 0; i < 32; i++) {
          preimageBytes[i] = random.nextInt(256);
        }
        final preimage = _bytesToHex(preimageBytes);
        paymentHash = _bytesToHex(sha256.convert(preimageBytes).bytes);
        print('HODL invoice preimage: $preimage');
        print('Payment hash: $paymentHash');
        print('Save the preimage! Use `claim-htlc-payment` with it to settle.');
      }
      paymentMethod = ReceivePaymentMethod.bolt11Invoice(
        description: description ?? '',
        amountSats: amount,
        expirySecs: expirySecs,
        paymentHash: paymentHash,
      );
    default:
      print('Invalid payment method: $method');
      return;
  }

  final result = await sdk.receivePayment(request: ReceivePaymentRequest(paymentMethod: paymentMethod));

  if (result.fee > BigInt.zero) {
    print('Prepared payment requires fee of ${result.fee} sats/token base units\n');
  }

  printValue(result);
}

// --- pay ---

Future<void> _handlePay(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final parser =
      _parser('pay')
        ..addOption('payment-request', abbr: 'r', mandatory: true, help: 'Invoice, address, or LNURL to pay')
        ..addOption('amount', abbr: 'a')
        ..addOption('token-identifier', abbr: 't')
        ..addOption('idempotency-key', abbr: 'i')
        ..addFlag('from-bitcoin', defaultsTo: false)
        ..addOption('from-token')
        ..addOption('convert-max-slippage-bps', abbr: 's')
        ..addFlag('fees-included', defaultsTo: false);
  final results = _parseArgs(parser, args, 'pay -r <request> [options]');
  if (results == null) return;

  final paymentRequest = results.option('payment-request')!;
  final amountStr = results.option('amount');
  final amount = amountStr != null ? BigInt.parse(amountStr) : null;
  final tokenIdentifier = results.option('token-identifier');
  final idempotencyKey = results.option('idempotency-key');
  final fromBitcoin = results.flag('from-bitcoin');
  final fromToken = results.option('from-token');
  final slippageStr = results.option('convert-max-slippage-bps');
  final slippage = slippageStr != null ? int.parse(slippageStr) : null;
  final feesIncluded = results.flag('fees-included');

  ConversionOptions? conversionOptions;
  if (fromBitcoin) {
    conversionOptions = ConversionOptions(
      conversionType: ConversionType.fromBitcoin(),
      maxSlippageBps: slippage,
      completionTimeoutSecs: null,
    );
  } else if (fromToken != null) {
    conversionOptions = ConversionOptions(
      conversionType: ConversionType.toBitcoin(fromTokenIdentifier: fromToken),
      maxSlippageBps: slippage,
      completionTimeoutSecs: null,
    );
  }

  final feePolicy = feesIncluded ? FeePolicy.feesIncluded : null;

  final prepareResponse = await sdk.prepareSendPayment(
    request: PrepareSendPaymentRequest(
      paymentRequest: PaymentRequest.input(input: paymentRequest),
      amount: amount,
      tokenIdentifier: tokenIdentifier,
      conversionOptions: conversionOptions,
      feePolicy: feePolicy,
    ),
  );

  if (prepareResponse.conversionEstimate != null) {
    final est = prepareResponse.conversionEstimate!;
    final units = est.options.conversionType is ConversionType_FromBitcoin ? 'sats' : 'token base units';
    print(
      'Estimated conversion of ${est.amountIn} $units → ${est.amountOut} $units with a ${est.fee} $units fee',
    );
    final answer = prompt('Do you want to continue (y/n): ', defaultValue: 'y');
    if (answer.toLowerCase() != 'y') {
      print('Payment cancelled');
      return;
    }
  }

  final paymentOptions = _readPaymentOptions(prepareResponse.paymentMethod);

  final sendResponse = await sdk.sendPayment(
    request: SendPaymentRequest(
      prepareResponse: prepareResponse,
      options: paymentOptions,
      idempotencyKey: idempotencyKey,
    ),
  );
  printValue(sendResponse);
}

// --- lnurl-pay ---

Future<void> _handleLnurlPay(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final parser =
      _parser('lnurl-pay')
        ..addOption('comment', abbr: 'c')
        ..addOption('validate', abbr: 'v')
        ..addOption('idempotency-key', abbr: 'i')
        ..addOption('from-token')
        ..addOption('convert-max-slippage-bps', abbr: 's')
        ..addFlag('fees-included', defaultsTo: false);
  final results = _parseArgs(parser, args, 'lnurl-pay <lnurl-or-address> [options]');
  if (results == null) return;

  // The LNURL is the first positional argument (remaining args)
  if (results.rest.isEmpty) {
    print('Usage: lnurl-pay <lnurl-or-lightning-address> [options]');
    return;
  }
  final lnurl = results.rest.first;

  final fromToken = results.option('from-token');
  final slippageStr = results.option('convert-max-slippage-bps');
  final slippage = slippageStr != null ? int.parse(slippageStr) : null;
  final feesIncluded = results.flag('fees-included');

  ConversionOptions? conversionOptions;
  if (fromToken != null) {
    conversionOptions = ConversionOptions(
      conversionType: ConversionType.toBitcoin(fromTokenIdentifier: fromToken),
      maxSlippageBps: slippage,
      completionTimeoutSecs: null,
    );
  }

  final feePolicy = feesIncluded ? FeePolicy.feesIncluded : null;

  final parsed = await sdk.parse(input: lnurl);

  LnurlPayRequestDetails payRequest;
  if (parsed is InputType_LightningAddress) {
    payRequest = parsed.field0.payRequest;
  } else if (parsed is InputType_LnurlPay) {
    payRequest = parsed.field0;
  } else {
    print('Invalid input: expected LNURL-pay or Lightning address');
    return;
  }

  final k = BigInt.from(1000);
  final minSendable = (payRequest.minSendable + k - BigInt.one) ~/ k;
  final maxSendable = payRequest.maxSendable ~/ k;
  final amountStr = prompt('Amount to pay (min $minSendable sat, max $maxSendable sat): ');
  final amountSats = BigInt.parse(amountStr);

  final prepareResponse = await sdk.prepareLnurlPay(
    request: PrepareLnurlPayRequest(
      amount: amountSats,
      comment: results.option('comment'),
      payRequest: payRequest,
      validateSuccessActionUrl: _parseBool(results.option('validate')),
      conversionOptions: conversionOptions,
      feePolicy: feePolicy,
    ),
  );

  if (prepareResponse.conversionEstimate != null) {
    final est = prepareResponse.conversionEstimate!;
    print(
      'Estimated conversion of ${est.amountIn} token base units → ${est.amountOut} sats with a ${est.fee} token base units fee',
    );
    final answer = prompt('Do you want to continue (y/n): ', defaultValue: 'y');
    if (answer.toLowerCase() != 'y') {
      print('Payment cancelled');
      return;
    }
  }

  printValue(prepareResponse);
  final answer = prompt('Do you want to continue? (y/n): ', defaultValue: 'y');
  if (answer.toLowerCase() != 'y') return;

  final result = await sdk.lnurlPay(
    request: LnurlPayRequest(
      prepareResponse: prepareResponse,
      idempotencyKey: results.option('idempotency-key'),
    ),
  );
  printValue(result);
}

// --- lnurl-withdraw ---

Future<void> _handleLnurlWithdraw(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final parser = _parser('lnurl-withdraw')..addOption('timeout', abbr: 't');
  final results = _parseArgs(parser, args, 'lnurl-withdraw <lnurl> [options]');
  if (results == null) return;

  if (results.rest.isEmpty) {
    print('Usage: lnurl-withdraw <lnurl> [options]');
    return;
  }
  final lnurl = results.rest.first;
  final timeoutStr = results.option('timeout');
  final timeout = timeoutStr != null ? int.parse(timeoutStr) : null;

  final parsed = await sdk.parse(input: lnurl);

  if (parsed is! InputType_LnurlWithdraw) {
    print('Invalid input: expected LNURL-withdraw');
    return;
  }

  final withdrawRequest = parsed.field0;
  final k = BigInt.from(1000);
  final minWithdrawable = (withdrawRequest.minWithdrawable + k - BigInt.one) ~/ k;
  final maxWithdrawable = withdrawRequest.maxWithdrawable ~/ k;
  final amountStr = prompt('Amount to withdraw (min $minWithdrawable sat, max $maxWithdrawable sat): ');
  final amountSats = BigInt.parse(amountStr);

  final result = await sdk.lnurlWithdraw(
    request: LnurlWithdrawRequest(
      amountSats: amountSats,
      withdrawRequest: withdrawRequest,
      completionTimeoutSecs: timeout,
    ),
  );
  printValue(result);
}

// --- lnurl-auth ---

Future<void> _handleLnurlAuth(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  if (args.isEmpty || args.first == 'help' || args.first == '--help') {
    print('Usage: lnurl-auth <lnurl>');
    return;
  }
  final lnurl = args.first;

  final parsed = await sdk.parse(input: lnurl);

  if (parsed is! InputType_LnurlAuth) {
    print('Invalid input: expected LNURL-auth');
    return;
  }

  final authRequest = parsed.field0;
  final action = authRequest.action ?? 'auth';
  final answer = prompt(
    'Authenticate with ${authRequest.domain} (action: $action)? (y/n): ',
    defaultValue: 'y',
  );
  if (answer.toLowerCase() != 'y') return;

  final result = await sdk.lnurlAuth(requestData: authRequest);
  printValue(result);
}

// --- claim-htlc-payment ---

Future<void> _handleClaimHtlcPayment(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  if (args.isEmpty || args.first == 'help' || args.first == '--help') {
    print('Usage: claim-htlc-payment <preimage>');
    return;
  }
  final preimage = args.first;
  final result = await sdk.claimHtlcPayment(request: ClaimHtlcPaymentRequest(preimage: preimage));
  printValue(result.payment);
}

// --- claim-deposit ---

Future<void> _handleClaimDeposit(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final parser =
      _parser('claim-deposit')
        ..addOption('fee-sat')
        ..addOption('sat-per-vbyte')
        ..addOption('recommended-fee-leeway');
  final results = _parseArgs(parser, args, 'claim-deposit <txid> <vout> [options]');
  if (results == null) return;

  if (results.rest.length < 2) {
    print('Usage: claim-deposit <txid> <vout> [options]');
    return;
  }
  final txid = results.rest[0];
  final vout = int.parse(results.rest[1]);

  final feeSatStr = results.option('fee-sat');
  final satPerVbyteStr = results.option('sat-per-vbyte');
  final leewayStr = results.option('recommended-fee-leeway');

  MaxFee? maxFee;
  if (leewayStr != null) {
    if (feeSatStr != null || satPerVbyteStr != null) {
      print('Cannot specify fee_sat or sat_per_vbyte when using recommended fee');
      return;
    }
    maxFee = MaxFee.networkRecommended(leewaySatPerVbyte: BigInt.parse(leewayStr));
  } else if (feeSatStr != null && satPerVbyteStr != null) {
    print('Cannot specify both fee_sat and sat_per_vbyte');
    return;
  } else if (feeSatStr != null) {
    maxFee = MaxFee.fixed(amount: BigInt.parse(feeSatStr));
  } else if (satPerVbyteStr != null) {
    maxFee = MaxFee.rate(satPerVbyte: BigInt.parse(satPerVbyteStr));
  }

  final result = await sdk.claimDeposit(request: ClaimDepositRequest(txid: txid, vout: vout, maxFee: maxFee));
  printValue(result);
}

// --- parse ---

Future<void> _handleParse(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  if (args.isEmpty || args.first == 'help' || args.first == '--help') {
    print('Usage: parse <input>');
    return;
  }
  final input = args.first;
  final result = await sdk.parse(input: input);
  printValue(result);
}

// --- refund-deposit ---

Future<void> _handleRefundDeposit(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final parser =
      _parser('refund-deposit')
        ..addOption('fee-sat')
        ..addOption('sat-per-vbyte');
  final results = _parseArgs(parser, args, 'refund-deposit <txid> <vout> <address> [options]');
  if (results == null) return;

  if (results.rest.length < 3) {
    print('Usage: refund-deposit <txid> <vout> <destination_address> [options]');
    return;
  }
  final txid = results.rest[0];
  final vout = int.parse(results.rest[1]);
  final destinationAddress = results.rest[2];

  final feeSatStr = results.option('fee-sat');
  final satPerVbyteStr = results.option('sat-per-vbyte');

  if (feeSatStr != null && satPerVbyteStr != null) {
    print('Cannot specify both fee_sat and sat_per_vbyte');
    return;
  }

  Fee fee;
  if (feeSatStr != null) {
    fee = Fee.fixed(amount: BigInt.parse(feeSatStr));
  } else if (satPerVbyteStr != null) {
    fee = Fee.rate(satPerVbyte: BigInt.parse(satPerVbyteStr));
  } else {
    print('Must specify either --fee-sat or --sat-per-vbyte');
    return;
  }

  final result = await sdk.refundDeposit(
    request: RefundDepositRequest(txid: txid, vout: vout, destinationAddress: destinationAddress, fee: fee),
  );
  printValue(result);
}

// --- list-unclaimed-deposits ---

Future<void> _handleListUnclaimedDeposits(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final result = await sdk.listUnclaimedDeposits(request: ListUnclaimedDepositsRequest());
  printValue(result);
}

// --- buy-bitcoin ---

Future<void> _handleBuyBitcoin(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final parser =
      _parser('buy-bitcoin')
        ..addOption('provider', defaultsTo: 'moonpay', help: 'Provider: moonpay (default) or cashapp')
        ..addOption('amount-sat', help: 'Amount in satoshis')
        ..addOption('redirect-url', help: 'Redirect URL after purchase (MoonPay only)');
  final results = _parseArgs(parser, args, 'buy-bitcoin [options]');
  if (results == null) return;

  final provider = results.option('provider')!.toLowerCase();
  final amountStr = results.option('amount-sat');
  final amount = amountStr != null ? BigInt.parse(amountStr) : null;
  final redirectUrl = results.option('redirect-url');

  BuyBitcoinRequest request;
  switch (provider) {
    case 'cashapp' || 'cash_app' || 'cash-app':
      request = BuyBitcoinRequest_CashApp(amountSats: amount);
    default:
      request = BuyBitcoinRequest_Moonpay(lockedAmountSat: amount, redirectUrl: redirectUrl);
  }

  final result = await sdk.buyBitcoin(request: request);
  print('Open this URL in a browser to complete the purchase:');
  print(result.url);
}

// --- check-lightning-address-available ---

Future<void> _handleCheckLightningAddressAvailable(
  BreezSdk sdk,
  TokenIssuer tokenIssuer,
  List<String> args,
) async {
  if (args.isEmpty || args.first == 'help' || args.first == '--help') {
    print('Usage: check-lightning-address-available <username>');
    return;
  }
  final username = args.first;
  final result = await sdk.checkLightningAddressAvailable(
    request: CheckLightningAddressRequest(username: username),
  );
  printValue(result);
}

// --- get-lightning-address ---

Future<void> _handleGetLightningAddress(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final result = await sdk.getLightningAddress();
  printValue(result);
}

// --- register-lightning-address ---

Future<void> _handleRegisterLightningAddress(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  if (args.isEmpty || args.first == 'help' || args.first == '--help') {
    print('Usage: register-lightning-address <username> [description]');
    return;
  }
  final username = args[0];
  final description = args.length > 1 ? args[1] : null;
  final result = await sdk.registerLightningAddress(
    request: RegisterLightningAddressRequest(username: username, description: description),
  );
  printValue(result);
}

// --- delete-lightning-address ---

Future<void> _handleDeleteLightningAddress(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  await sdk.deleteLightningAddress();
  print('Lightning address deleted');
}

// --- list-fiat-currencies ---

Future<void> _handleListFiatCurrencies(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final result = await sdk.listFiatCurrencies();
  printValue(result);
}

// --- list-fiat-rates ---

Future<void> _handleListFiatRates(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final result = await sdk.listFiatRates();
  printValue(result);
}

// --- recommended-fees ---

Future<void> _handleRecommendedFees(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final result = await sdk.recommendedFees();
  printValue(result);
}

// --- get-tokens-metadata ---

Future<void> _handleGetTokensMetadata(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  if (args.isEmpty || args.first == 'help' || args.first == '--help') {
    print('Usage: get-tokens-metadata <token_identifier> [token_identifier...]');
    return;
  }
  final result = await sdk.getTokensMetadata(request: GetTokensMetadataRequest(tokenIdentifiers: args));
  printValue(result);
}

// --- fetch-conversion-limits ---

Future<void> _handleFetchConversionLimits(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final parser = _parser('fetch-conversion-limits')..addFlag('from-bitcoin', abbr: 'f', defaultsTo: false);
  final results = _parseArgs(parser, args, 'fetch-conversion-limits [-f] <token_identifier>');
  if (results == null) return;

  if (results.rest.isEmpty) {
    print('Usage: fetch-conversion-limits [-f] <token_identifier>');
    return;
  }
  final tokenIdentifier = results.rest.first;
  final fromBitcoin = results.flag('from-bitcoin');

  FetchConversionLimitsRequest request;
  if (fromBitcoin) {
    request = FetchConversionLimitsRequest(
      conversionType: ConversionType.fromBitcoin(),
      tokenIdentifier: tokenIdentifier,
    );
  } else {
    request = FetchConversionLimitsRequest(
      conversionType: ConversionType.toBitcoin(fromTokenIdentifier: tokenIdentifier),
      tokenIdentifier: null,
    );
  }

  final result = await sdk.fetchConversionLimits(request: request);
  printValue(result);
}

// --- get-user-settings ---

Future<void> _handleGetUserSettings(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final result = await sdk.getUserSettings();
  printValue(result);
}

// --- set-user-settings ---

Future<void> _handleSetUserSettings(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final parser = _parser('set-user-settings')..addOption('private', abbr: 'p');
  final results = _parseArgs(parser, args, 'set-user-settings [options]');
  if (results == null) return;
  final privateMode = _parseBool(results.option('private'));

  await sdk.updateUserSettings(request: UpdateUserSettingsRequest(sparkPrivateModeEnabled: privateMode));
  print('User settings updated');
}

// --- get-spark-status ---

Future<void> _handleGetSparkStatus(BreezSdk sdk, TokenIssuer tokenIssuer, List<String> args) async {
  final result = await getSparkStatus();
  printValue(result);
}

// ---------------------------------------------------------------------------
// read_payment_options — interactive fee/option selection
// ---------------------------------------------------------------------------

SendPaymentOptions? _readPaymentOptions(SendPaymentMethod paymentMethod) {
  if (paymentMethod is SendPaymentMethod_BitcoinAddress) {
    final feeQuote = paymentMethod.feeQuote;
    final fastFee = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat;
    final mediumFee = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat;
    final slowFee = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat;
    print('Please choose payment fee:');
    print('1. Fast: $fastFee');
    print('2. Medium: $mediumFee');
    print('3. Slow: $slowFee');
    final line = prompt('', defaultValue: '1');
    OnchainConfirmationSpeed speed;
    switch (line) {
      case '2':
        speed = OnchainConfirmationSpeed.medium;
      case '3':
        speed = OnchainConfirmationSpeed.slow;
      default:
        speed = OnchainConfirmationSpeed.fast;
    }
    return SendPaymentOptions.bitcoinAddress(confirmationSpeed: speed);
  }

  if (paymentMethod is SendPaymentMethod_Bolt11Invoice) {
    if (paymentMethod.sparkTransferFeeSats != null) {
      print('Choose payment option:');
      print('1. Spark transfer fee: ${paymentMethod.sparkTransferFeeSats} sats');
      print('2. Lightning fee: ${paymentMethod.lightningFeeSats} sats');
      final line = prompt('', defaultValue: '1');
      if (line == '1') {
        return SendPaymentOptions.bolt11Invoice(preferSpark: true, completionTimeoutSecs: 0);
      }
    }
    return SendPaymentOptions.bolt11Invoice(preferSpark: false, completionTimeoutSecs: 0);
  }

  if (paymentMethod is SendPaymentMethod_SparkAddress) {
    // HTLC options are only valid for Bitcoin payments, not token payments
    if (paymentMethod.tokenIdentifier != null) return null;

    final answer = prompt('Do you want to create an HTLC transfer? (y/n)', defaultValue: 'n');
    if (answer.toLowerCase() != 'y') return null;

    var paymentHash = prompt('Please enter the HTLC payment hash (hex string) or leave empty to generate: ');
    if (paymentHash.isEmpty) {
      final random = Random.secure();
      final preimageBytes = Uint8List(32);
      for (var i = 0; i < 32; i++) {
        preimageBytes[i] = random.nextInt(256);
      }
      final preimage = _bytesToHex(preimageBytes);
      paymentHash = _bytesToHex(sha256.convert(preimageBytes).bytes);
      print('Generated preimage: $preimage');
      print('Associated payment hash: $paymentHash');
    }

    final expiryStr = prompt('Please enter the HTLC expiry duration in seconds: ');
    final expiryDurationSecs = BigInt.parse(expiryStr);

    return SendPaymentOptions.sparkAddress(
      htlcOptions: SparkHtlcOptions(paymentHash: paymentHash, expiryDurationSecs: expiryDurationSecs),
    );
  }

  // SendPaymentMethod_SparkInvoice
  return null;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

String _bytesToHex(List<int> bytes) {
  final sb = StringBuffer();
  for (final b in bytes) {
    sb.write(b.toRadixString(16).padLeft(2, '0'));
  }
  return sb.toString();
}
