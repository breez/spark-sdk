import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'package:breez_sdk_spark_flutter/webln.dart';
import 'package:flutter_inappwebview/flutter_inappwebview.dart';

// ANCHOR: webln-integration
Future<void> setupWebLn(
  BreezSdk sdk,
  InAppWebViewController webViewController,
) async {
  // Create the WebLN controller after the WebView is ready
  final weblnController = WebLnController(
    sdk: sdk,
    webViewController: webViewController,
    onEnableRequest: (domain) async {
      // Show a dialog asking the user to approve WebLN access
      // Return true to allow, false to deny
      return await showEnableDialog(domain);
    },
    onPaymentRequest: (invoice, amountSats) async {
      // Show a dialog asking the user to approve the payment
      // Return true to approve, false to reject
      return await showPaymentDialog(invoice, amountSats);
    },
    onLnurlRequest: (request) async {
      // Handle LNURL requests (pay, withdraw, auth)
      switch (request.type) {
        case LnurlType.pay:
          // Show UI to select amount within min/max bounds
          // Return LnurlUserResponse with approved, amountSats, and optional comment
          return const LnurlUserResponse(approved: true, amountSats: 1000);
        case LnurlType.withdraw:
          // Show UI to select amount within min/max bounds
          return const LnurlUserResponse(approved: true, amountSats: 1000);
        case LnurlType.auth:
          // Show confirmation dialog
          return const LnurlUserResponse(approved: true);
      }
    },
  );

  // Inject the WebLN provider into the WebView
  await weblnController.inject();
}
// ANCHOR_END: webln-integration

// Placeholder functions - implement these with your UI framework
Future<bool> showEnableDialog(String domain) async {
  // Show your own permission dialog here
  return true;
}

Future<bool> showPaymentDialog(String invoice, int amountSats) async {
  // Show your own payment confirmation dialog here
  return true;
}
