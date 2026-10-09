import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> handleErrors(BreezSdk sdk, PrepareSendPaymentRequest request) async {
  // ANCHOR: handle-errors
  try {
    final prepareResponse = await sdk.prepareSendPayment(request: request);
    print('Payment prepared: ${prepareResponse.paymentMethod}');
  } on SdkError_InsufficientFunds {
    print('Not enough funds for this payment');
  } on SdkError_CrossChainDisabled catch (e) {
    print('Cross-chain payments are not enabled, see ${e.docsUrl}');
  } catch (e) {
    print('Failed to prepare the payment: $e');
  }
  // ANCHOR_END: handle-errors
}
