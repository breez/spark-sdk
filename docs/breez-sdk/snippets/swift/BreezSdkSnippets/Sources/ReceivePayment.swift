import BreezSdkSpark

func receiveLightning(sdk: BreezSdk) async throws -> ReceivePaymentResponse {
    // ANCHOR: receive-payment-lightning-bolt11
    let description = "<invoice description>"
    // Optionally set the invoice amount you wish the payer to send
    let optionalAmountSats = 5_000
    let response = try await sdk
        .receivePayment(request: ReceivePaymentRequest(
            paymentMethod: ReceivePaymentMethod.bolt11Invoice(
                description: description,
                amountSats: optionalAmountSats
            )
        ));

    let paymentRequest = response.paymentRequest;
    print("Payment Request: {}", paymentRequest);
    let receiveFeeSats = response.feeSats;
    print("Fees: {} sats", receiveFeeSats);
    // ANCHOR_END: receive-payment-lightning-bolt11

    return response
}

func receiveOnchain(sdk: BreezSdk) async throws -> ReceivePaymentResponse {
    // ANCHOR: receive-payment-onchain
    let response = try await sdk
        .receivePayment(request: ReceivePaymentRequest(
            paymentMethod: ReceivePaymentMethod.bitcoinAddress
        ));

    let paymentRequest = response.paymentRequest;
    print("Payment Request: {}", paymentRequest);
    let receiveFeeSats = response.feeSats;
    print("Fees: {} sats", receiveFeeSats);
    // ANCHOR_END: receive-payment-onchain

    return response
}

func receiveSpark(sdk: BreezSdk) async throws -> ReceivePaymentResponse {
    // ANCHOR: receive-payment-spark
    let response = try await sdk
        .receivePayment(request: ReceivePaymentRequest(
            paymentMethod: ReceivePaymentMethod.sparkAddress
        ));

    let paymentRequest = response.paymentRequest;
    print("Payment Request: {}", paymentRequest);
    let receiveFeeSats = response.feeSats;
    print("Fees: {} sats", receiveFeeSats);
    // ANCHOR_END: receive-payment-spark

    return response
}
