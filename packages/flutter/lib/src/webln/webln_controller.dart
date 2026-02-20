/// WebLn controller for Flutter WebViews
///
/// This controller integrates with `flutter_inappwebview`
/// to provide WebLN support in WebViews.
library;

import 'dart:convert';
import 'dart:developer' as developer;

import 'package:flutter_inappwebview/flutter_inappwebview.dart';

import '../rust/sdk.dart';
import '../rust/models.dart';
import '../rust/errors.dart';
import 'provider_script.dart';
import 'types.dart';

/// Callback for enable requests.
/// Called when a website requests WebLN access.
///
/// [domain] is the domain requesting access.
/// Return `true` to allow access, `false` to deny.
typedef OnEnableRequest = Future<bool> Function(String domain);

/// Callback for payment requests.
/// Called when a website requests to send a payment.
///
/// [invoice] is the BOLT11 invoice.
/// [amountSats] is the amount in satoshis.
/// Return `true` to approve payment, `false` to reject.
typedef OnPaymentRequest = Future<bool> Function(String invoice, int amountSats);

/// Callback for LNURL requests.
/// Called when a website initiates an LNURL flow.
///
/// [request] contains the LNURL request details.
/// Return user's response with approval and optional amount/comment.
typedef OnLnurlRequest = Future<LnurlUserResponse> Function(LnurlRequest request);

/// Controller for WebLN integration in Flutter WebViews
///
/// This controller handles the communication between WebLN-aware websites
/// and the Breez Spark SDK through a WebView bridge.
///
/// Example usage:
/// ```dart
/// final controller = WebLnController(
///   sdk: sdk,
///   webViewController: webViewController,
///   onEnableRequest: (domain) async {
///     return await showDialog<bool>(...) ?? false;
///   },
///   onPaymentRequest: (invoice, amountSats) async {
///     return await showDialog<bool>(...) ?? false;
///   },
///   onLnurlRequest: (request) async {
///     return await showLnurlDialog(request);
///   },
/// );
///
/// await controller.inject();
/// ```
class WebLnController {
  /// The Breez SDK instance
  final BreezSdk sdk;

  /// The WebView controller
  final InAppWebViewController webViewController;

  /// Callback invoked when a website requests WebLN access
  final OnEnableRequest onEnableRequest;

  /// Callback invoked when a website requests a payment
  final OnPaymentRequest onPaymentRequest;

  /// Callback invoked for LNURL operations
  final OnLnurlRequest onLnurlRequest;

  /// Set of domains that have been enabled
  final Set<String> _enabledDomains = {};

  /// Cached node pubkey (retrieved via signMessage)
  String? _cachedPubkey;

  /// Supported WebLN methods
  static const List<String> _supportedMethods = [
    'getInfo',
    'sendPayment',
    'makeInvoice',
    'signMessage',
    'verifyMessage',
    'lnurl',
  ];

  WebLnController({
    required this.sdk,
    required this.webViewController,
    required this.onEnableRequest,
    required this.onPaymentRequest,
    required this.onLnurlRequest,
  });

  /// Injects the WebLN provider script into the WebView
  Future<void> inject() async {
    // Add the JavaScript handler for receiving messages
    webViewController.addJavaScriptHandler(
      handlerName: 'BreezSparkWebLn',
      callback: (args) async {
        if (args.isEmpty) return;
        final json = args[0] as String;
        await _handleMessage(json);
      },
    );

    // Inject the provider script
    await webViewController.evaluateJavascript(source: weblnProviderScript);
  }

  /// Handles incoming WebLN requests from the WebView
  Future<void> _handleMessage(String json) async {
    try {
      final request = jsonDecode(json) as Map<String, dynamic>;
      final id = request['id'] as String;
      final method = request['method'] as String;
      final params = request['params'] as Map<String, dynamic>? ?? {};

      switch (method) {
        case 'enable':
          await _handleEnable(id, params);
          break;
        case 'getInfo':
          await _handleGetInfo(id);
          break;
        case 'sendPayment':
          await _handleSendPayment(id, params);
          break;
        case 'makeInvoice':
          await _handleMakeInvoice(id, params);
          break;
        case 'signMessage':
          await _handleSignMessage(id, params);
          break;
        case 'verifyMessage':
          await _handleVerifyMessage(id, params);
          break;
        case 'lnurl':
          await _handleLnurl(id, params);
          break;
        default:
          await _respond(id, error: WebLnErrorCode.unsupportedMethod);
      }
    } catch (e) {
      // Log error but don't crash
      developer.log('WebLN error: $e', name: 'BreezSparkWebLn');
    }
  }

  Future<void> _handleEnable(String id, Map<String, dynamic> params) async {
    final domain = params['domain'] as String?;
    if (domain == null) {
      await _respond(id, error: WebLnErrorCode.invalidParams);
      return;
    }

    // Check if already enabled
    if (_enabledDomains.contains(domain)) {
      await _respond(id, result: {});
      return;
    }

    // Request permission from user
    final approved = await onEnableRequest(domain);
    if (approved) {
      _enabledDomains.add(domain);
      await _respond(id, result: {});
    } else {
      await _respond(id, error: WebLnErrorCode.userRejected);
    }
  }

  Future<void> _handleGetInfo(String id) async {
    try {
      // Get pubkey by signing a message (SDK doesn't expose pubkey via getInfo)
      final pubkey = await _getNodePubkey();

      await _respond(
        id,
        result: {
          'node': {'pubkey': pubkey, 'alias': ''},
          'methods': _supportedMethods,
        },
      );
    } catch (e) {
      await _respond(id, error: WebLnErrorCode.internalError);
    }
  }

  /// Gets the node pubkey by signing a message
  Future<String> _getNodePubkey() async {
    if (_cachedPubkey != null) {
      return _cachedPubkey!;
    }

    final response = await sdk.signMessage(
      request: const SignMessageRequest(message: 'webln_pubkey_request', compact: true),
    );
    _cachedPubkey = response.pubkey;
    return response.pubkey;
  }

  Future<void> _handleSendPayment(String id, Map<String, dynamic> params) async {
    final paymentRequest = params['paymentRequest'] as String?;
    if (paymentRequest == null) {
      await _respond(id, error: WebLnErrorCode.invalidParams);
      return;
    }

    try {
      // Parse the invoice to get amount
      final parsed = await sdk.parse(input: paymentRequest);
      int amountSats = 0;

      if (parsed is InputType_Bolt11Invoice) {
        final invoiceDetails = parsed.field0;
        if (invoiceDetails.amountMsat != null) {
          amountSats = (invoiceDetails.amountMsat!.toInt() / 1000).round();
        }
      }

      // Request payment confirmation from user
      final approved = await onPaymentRequest(paymentRequest, amountSats);
      if (!approved) {
        await _respond(id, error: WebLnErrorCode.userRejected);
        return;
      }

      // Prepare and send payment
      final prepared = await sdk.prepareSendPayment(
        request: PrepareSendPaymentRequest(paymentRequest: paymentRequest),
      );

      final result = await sdk.sendPayment(
        request: SendPaymentRequest(
          prepareResponse: prepared,
          options: SendPaymentOptions_Bolt11Invoice(preferSpark: false, completionTimeoutSecs: 60),
        ),
      );

      // Extract preimage from payment details
      String preimage = '';
      final details = result.payment.details;
      if (details is PaymentDetails_Lightning) {
        preimage = details.htlcDetails.preimage ?? '';
      }

      await _respond(id, result: {'preimage': preimage});
    } on SdkError catch (e) {
      if (e is SdkError_InsufficientFunds) {
        await _respond(id, error: WebLnErrorCode.insufficientFunds);
      } else {
        await _respond(id, error: WebLnErrorCode.internalError);
      }
    } catch (e) {
      await _respond(id, error: WebLnErrorCode.internalError);
    }
  }

  Future<void> _handleMakeInvoice(String id, Map<String, dynamic> params) async {
    try {
      final amount = params['amount'] ?? params['defaultAmount'];
      final memo = params['defaultMemo'] as String?;

      BigInt? amountSats;
      if (amount != null) {
        if (amount is int) {
          amountSats = BigInt.from(amount);
        } else if (amount is String) {
          final parsed = int.tryParse(amount);
          if (parsed != null) {
            amountSats = BigInt.from(parsed);
          }
        }
      }

      final response = await sdk.receivePayment(
        request: ReceivePaymentRequest(
          paymentMethod: ReceivePaymentMethod_Bolt11Invoice(amountSats: amountSats, description: memo ?? ''),
        ),
      );

      await _respond(id, result: {'paymentRequest': response.paymentRequest});
    } catch (e) {
      await _respond(id, error: WebLnErrorCode.internalError);
    }
  }

  Future<void> _handleSignMessage(String id, Map<String, dynamic> params) async {
    final message = params['message'] as String?;
    if (message == null) {
      await _respond(id, error: WebLnErrorCode.invalidParams);
      return;
    }

    try {
      final response = await sdk.signMessage(request: SignMessageRequest(message: message, compact: true));
      await _respond(id, result: {'message': message, 'signature': response.signature});
    } catch (e) {
      await _respond(id, error: WebLnErrorCode.internalError);
    }
  }

  Future<void> _handleVerifyMessage(String id, Map<String, dynamic> params) async {
    final signature = params['signature'] as String?;
    final message = params['message'] as String?;

    if (signature == null || message == null) {
      await _respond(id, error: WebLnErrorCode.invalidParams);
      return;
    }

    try {
      final pubkey = await _getNodePubkey();
      final response = await sdk.checkMessage(
        request: CheckMessageRequest(message: message, pubkey: pubkey, signature: signature),
      );

      if (response.isValid) {
        await _respond(id, result: {});
      } else {
        await _respond(id, error: WebLnErrorCode.invalidParams);
      }
    } catch (e) {
      await _respond(id, error: WebLnErrorCode.internalError);
    }
  }

  Future<void> _handleLnurl(String id, Map<String, dynamic> params) async {
    final lnurlString = params['lnurl'] as String?;
    if (lnurlString == null) {
      await _respond(id, error: WebLnErrorCode.invalidParams);
      return;
    }

    try {
      final parsed = await sdk.parse(input: lnurlString);

      if (parsed is InputType_LnurlPay) {
        await _handleLnurlPay(id, parsed.field0);
      } else if (parsed is InputType_LnurlWithdraw) {
        await _handleLnurlWithdraw(id, parsed.field0);
      } else if (parsed is InputType_LnurlAuth) {
        await _handleLnurlAuth(id, parsed.field0);
      } else {
        await _respond(id, error: WebLnErrorCode.invalidParams);
      }
    } catch (e) {
      await _respond(id, error: WebLnErrorCode.internalError);
    }
  }

  Future<void> _handleLnurlPay(String id, LnurlPayRequestDetails data) async {
    final lnurlResponse = await onLnurlRequest(
      LnurlRequest(
        type: LnurlType.pay,
        domain: data.domain,
        minAmountSats: (data.minSendable.toInt() / 1000).round(),
        maxAmountSats: (data.maxSendable.toInt() / 1000).round(),
        metadata: data.metadataStr,
      ),
    );

    if (!lnurlResponse.approved) {
      await _respond(id, error: WebLnErrorCode.userRejected);
      return;
    }

    try {
      final prepared = await sdk.prepareLnurlPay(
        request: PrepareLnurlPayRequest(
          payRequest: data,
          amountSats: BigInt.from(lnurlResponse.amountSats ?? 0),
          comment: lnurlResponse.comment,
        ),
      );

      final result = await sdk.lnurlPay(request: LnurlPayRequest(prepareResponse: prepared));

      // Extract preimage from payment details
      String preimage = '';
      final details = result.payment.details;
      if (details is PaymentDetails_Lightning) {
        preimage = details.htlcDetails.preimage ?? '';
      }

      await _respond(id, result: {'status': 'OK', 'preimage': preimage});
    } on SdkError catch (e) {
      if (e is SdkError_InsufficientFunds) {
        await _respond(id, error: WebLnErrorCode.insufficientFunds);
      } else {
        await _respond(id, error: WebLnErrorCode.internalError);
      }
    } catch (e) {
      await _respond(id, error: WebLnErrorCode.internalError);
    }
  }

  Future<void> _handleLnurlWithdraw(String id, LnurlWithdrawRequestDetails data) async {
    final lnurlResponse = await onLnurlRequest(
      LnurlRequest(
        type: LnurlType.withdraw,
        domain: data.callback.split('/').length > 2 ? Uri.parse(data.callback).host : data.callback,
        minAmountSats: (data.minWithdrawable.toInt() / 1000).round(),
        maxAmountSats: (data.maxWithdrawable.toInt() / 1000).round(),
        defaultDescription: data.defaultDescription,
      ),
    );

    if (!lnurlResponse.approved) {
      await _respond(id, error: WebLnErrorCode.userRejected);
      return;
    }

    try {
      await sdk.lnurlWithdraw(
        request: LnurlWithdrawRequest(
          withdrawRequest: data,
          amountSats: BigInt.from(lnurlResponse.amountSats ?? 0),
        ),
      );
      await _respond(id, result: {'status': 'OK'});
    } catch (e) {
      await _respond(id, error: WebLnErrorCode.internalError);
    }
  }

  Future<void> _handleLnurlAuth(String id, LnurlAuthRequestDetails data) async {
    final lnurlResponse = await onLnurlRequest(LnurlRequest(type: LnurlType.auth, domain: data.domain));

    if (!lnurlResponse.approved) {
      await _respond(id, error: WebLnErrorCode.userRejected);
      return;
    }

    try {
      await sdk.lnurlAuth(requestData: data);
      await _respond(id, result: {'status': 'OK'});
    } catch (e) {
      await _respond(id, error: WebLnErrorCode.internalError);
    }
  }

  /// Sends a response back to the WebView
  Future<void> _respond(String id, {Map<String, dynamic>? result, String? error}) async {
    final response = {
      'id': id,
      'success': error == null,
      if (result != null) 'result': result,
      if (error != null) 'error': error,
    };

    final responseJson = jsonEncode(response);
    await webViewController.evaluateJavascript(
      source: 'window.__breezSparkWebLnHandleResponse($responseJson);',
    );
  }
}
