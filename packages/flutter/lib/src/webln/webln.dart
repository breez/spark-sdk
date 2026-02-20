/// WebLn support for Flutter WebViews
///
/// This module provides WebLN integration for Flutter apps using
/// `flutter_inappwebview`. It allows WebLN-aware websites to interact
/// with the Breez Spark SDK through a WebView bridge.
///
/// ## Usage
///
/// ```dart
/// import 'package:breez_sdk_spark_flutter/webln.dart';
///
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
library;

export 'types.dart';
export 'webln_controller.dart';
