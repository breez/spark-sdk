import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

X402 getX402(BreezSdk sdk) {
  // ANCHOR: get-x402
  X402 x402 = sdk.getX402();
  // ANCHOR_END: get-x402
  return x402;
}

Future<void> payHttpRequest(X402 x402, String paymentRequiredHeader) async {
  // ANCHOR: pay-http-request
  // The request that returned 402, exactly as it will be sent again
  X402Request request = X402Request.http(
    method: "GET",
    url: "https://api.example.com/article/A",
    headers: [X402Header(name: "accept", value: "application/json")],
    body: null,
  );
  PayX402Response response = await x402.pay(
    request: PayX402Request(
      // The PAYMENT-REQUIRED header of the 402 response
      paymentRequired: paymentRequiredHeader,
      request: request,
      maxAmount: X402PaymentAmount.lightning(amountSats: BigInt.from(100)),
      conversionOptions: null,
      completionTimeoutSecs: null,
    ),
  );

  String? paymentSignature = response.paymentSignature;
  if (paymentSignature != null) {
    // Send the request again with a PAYMENT-SIGNATURE header
    print("PAYMENT-SIGNATURE: $paymentSignature");
  } else {
    // Still in flight: paying the same challenge again picks it up
    print("Payment status: ${response.status}");
  }
  // ANCHOR_END: pay-http-request
}

Future<void> payMcpToolCall(X402 x402, String structuredContent) async {
  // ANCHOR: pay-mcp-tool-call
  X402Request request = X402Request.mcp(
    server: "https://api.example.com/mcp",
    // The tools/call params, exactly as they will be sent again
    params: '{"name":"get_article","arguments":{"article":"A"}}',
  );
  PayX402Response response = await x402.pay(
    request: PayX402Request(
      // The structuredContent of the tool result
      paymentRequired: structuredContent,
      request: request,
      maxAmount: X402PaymentAmount.lightning(amountSats: BigInt.from(100)),
      conversionOptions: null,
      completionTimeoutSecs: null,
    ),
  );

  String? paymentSignature = response.paymentSignature;
  if (paymentSignature != null) {
    // Call the tool again with this JSON in params._meta["x402/payment"]
    print("Payment: $paymentSignature");
  }
  // ANCHOR_END: pay-mcp-tool-call
}
