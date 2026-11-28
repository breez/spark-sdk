import BreezSdkSpark

func getX402(sdk: BreezSdk) -> X402 {
    // ANCHOR: get-x402
    let x402 = sdk.getX402()
    // ANCHOR_END: get-x402
    return x402
}

func payHttpRequest(x402: X402, paymentRequiredHeader: String) async throws {
    // ANCHOR: pay-http-request
    // The request that returned 402, exactly as it will be sent again
    let request = X402Request.http(
        method: "GET",
        url: "https://api.example.com/article/A",
        headers: [X402Header(name: "accept", value: "application/json")],
        body: nil
    )
    let response = try await x402.pay(
        request: PayX402Request(
            // The PAYMENT-REQUIRED header of the 402 response
            paymentRequired: paymentRequiredHeader,
            request: request,
            maxAmount: X402PaymentAmount.lightning(amountSats: 100),
            conversionOptions: nil,
            completionTimeoutSecs: nil
        ))

    if response.paymentSignature != nil {
        // Send the request again with it as the PAYMENT-SIGNATURE header.
        // It proves the payment, so don't log it.
    } else {
        // Still in flight: paying the same challenge again picks it up
        print("Payment status: \(response.status)")
    }
    // ANCHOR_END: pay-http-request
}

func payMcpToolCall(x402: X402, structuredContent: String) async throws {
    // ANCHOR: pay-mcp-tool-call
    let request = X402Request.mcp(
        server: "https://api.example.com/mcp",
        // The tools/call params, exactly as they will be sent again
        params: #"{"name":"get_article","arguments":{"article":"A"}}"#
    )
    let response = try await x402.pay(
        request: PayX402Request(
            // The structuredContent of the tool result
            paymentRequired: structuredContent,
            request: request,
            maxAmount: X402PaymentAmount.lightning(amountSats: 100),
            conversionOptions: nil,
            completionTimeoutSecs: nil
        ))

    if response.paymentSignature != nil {
        // Call the tool again with this JSON in params._meta["x402/payment"]
    }
    // ANCHOR_END: pay-mcp-tool-call
}

func createChallenge(x402: X402) async throws {
    // ANCHOR: create-challenge
    // The unpaid request, with its URL built from your public origin
    let request = X402Request.http(
        method: "GET",
        url: "https://api.example.com/article/A",
        headers: [X402Header(name: "accept", value: "application/json")],
        body: nil
    )
    let response = try await x402.createChallenge(
        request: CreateX402ChallengeRequest(
            request: request,
            // Headers that change what is served, so the payment covers them too
            boundNames: ["accept"],
            amounts: [X402PaymentAmount.lightning(amountSats: 21)],
            maxTimeoutSecs: nil,
            description: "An article",
            mimeType: "application/json"
        ))

    // Respond with status 402 and this PAYMENT-REQUIRED header
    print("PAYMENT-REQUIRED: \(response.paymentRequired)")
    // ANCHOR_END: create-challenge
}

func settlePayment(x402: X402, paymentSignatureHeader: String) async throws {
    // ANCHOR: settle-payment
    // The paid request, exactly as received
    let request = X402Request.http(
        method: "GET",
        url: "https://api.example.com/article/A",
        headers: [X402Header(name: "accept", value: "application/json")],
        body: nil
    )
    let response = try await x402.settle(
        request: SettleX402Request(
            // The PAYMENT-SIGNATURE header of the request
            paymentSignature: paymentSignatureHeader,
            request: request,
            boundNames: ["accept"],
            // The same price and lifetime the challenge was created with
            amounts: [X402PaymentAmount.lightning(amountSats: 21)],
            maxTimeoutSecs: nil
        ))

    // Send the PAYMENT-RESPONSE header whatever the status
    print("PAYMENT-RESPONSE: \(response.paymentResponse)")
    switch response.status {
    case .settled:
        // Serve the resource
        break
    case .pending:
        // Respond with status 402 and no new challenge
        break
    case .malformed:
        // Respond with status 400
        break
    case .refused:
        // Respond with status 402 and a new challenge
        break
    case .error:
        // Respond with status 500
        break
    }
    // ANCHOR_END: settle-payment
}
