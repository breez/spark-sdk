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

  if (response.paymentSignature != null) {
    // Send the request again with it as the PAYMENT-SIGNATURE header.
    // It proves the payment, so don't log it.
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

  if (response.paymentSignature != null) {
    // Call the tool again with this JSON in params._meta["x402/payment"]
  }
  // ANCHOR_END: pay-mcp-tool-call
}

Future<void> createChallenge(X402 x402) async {
  // ANCHOR: create-challenge
  // The unpaid request, with its URL built from your public origin
  X402Request request = X402Request.http(
    method: "GET",
    url: "https://api.example.com/article/A",
    headers: [X402Header(name: "accept", value: "application/json")],
    body: null,
  );
  CreateX402ChallengeResponse response = await x402.createChallenge(
    request: CreateX402ChallengeRequest(
      request: request,
      // Headers that change what is served, so the payment covers them too
      boundNames: ["accept"],
      amounts: [X402PaymentAmount.lightning(amountSats: BigInt.from(21))],
      maxTimeoutSecs: null,
      description: "An article",
      mimeType: "application/json",
    ),
  );

  // Respond with status 402 and this PAYMENT-REQUIRED header
  print("PAYMENT-REQUIRED: ${response.paymentRequired}");
  // ANCHOR_END: create-challenge
}

Future<void> settlePayment(X402 x402, String paymentSignatureHeader) async {
  // ANCHOR: settle-payment
  // The paid request, exactly as received
  X402Request request = X402Request.http(
    method: "GET",
    url: "https://api.example.com/article/A",
    headers: [X402Header(name: "accept", value: "application/json")],
    body: null,
  );
  SettleX402Response response = await x402.settle(
    request: SettleX402Request(
      // The PAYMENT-SIGNATURE header of the request
      paymentSignature: paymentSignatureHeader,
      request: request,
      boundNames: ["accept"],
      // The same price and lifetime the challenge was created with
      amounts: [X402PaymentAmount.lightning(amountSats: BigInt.from(21))],
      maxTimeoutSecs: null,
    ),
  );

  // Send the PAYMENT-RESPONSE header whatever the status
  print("PAYMENT-RESPONSE: ${response.paymentResponse}");
  switch (response.status) {
    case X402SettleStatus.settled:
      // Serve the resource
      break;
    case X402SettleStatus.pending:
      // Respond with status 402 and no new challenge
      break;
    case X402SettleStatus.malformed:
      // Respond with status 400
      break;
    case X402SettleStatus.refused:
      // Respond with status 402 and a new challenge
      break;
    case X402SettleStatus.error:
      // Respond with status 500
      break;
  }
  // ANCHOR_END: settle-payment
}
