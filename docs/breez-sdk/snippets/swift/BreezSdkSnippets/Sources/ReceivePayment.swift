import BreezSdkSpark

func prepareReceiveLightning(sdk: BreezSdk) throws -> PrepareReceiveResponse {
    // ANCHOR: prepare-receive-payment-lightning
    let description = "<invoice description>"
    // Optionally set the invoice amount you wish the payer to send
    let optionalAmountSats = 5_000
    let prepareResponse = try sdk
        .prepareReceivePayment(request: PrepareReceivePaymentRequest(
            paymentMethod: ReceivePaymentMethod.bolt11Invoice(description, optionalAmountSats)
        ));

    let receiveFeeSats = prepareResponse.feeSats;
    print("Fees: {} sats", receiveFeeSats);
    // ANCHOR_END: prepare-receive-payment-lightning

    return prepareResponse
}

func prepareReceiveOnchain(sdk: BreezSdk) throws -> PrepareReceiveResponse {
    // ANCHOR: prepare-receive-payment-onchain
    let prepareResponse = try sdk
        .prepareReceivePayment(request: PrepareReceivePaymentRequest(
            paymentMethod: ReceivePaymentMethod.bitcoinAddress
        ));

    let receiveFeeSats = prepareResponse.feeSats;
    print("Fees: {} sats", receiveFeeSats);    // ANCHOR_END: prepare-receive-payment-onchain

    return prepareResponse
}

func prepareReceiveSpark(sdk: BreezSdk) throws -> PrepareReceiveResponse {
    // ANCHOR: prepare-receive-payment-spark
    let prepareResponse = try sdk
        .prepareReceivePayment(request: PrepareReceivePaymentRequest(
            paymentMethod: ReceivePaymentMethod.sparkAddress
        ));

    let receiveFeeSats = prepareResponse.feeSats;
    print("Fees: {} sats", receiveFeeSats);
    // ANCHOR_END: prepare-receive-payment-spark

    return prepareResponse
}

func receivePayment(sdk: BreezSdk, prepareResponse: PrepareReceivePaymentResponse) async throws -> ReceivePaymentResponse {
    // ANCHOR: receive-payment
    let response = try await sdk.receivePayment(request: ReceivePaymentRequest(
        prepareResponse: prepareResponse,
    ))

    let paymentRequest: String = response.paymentRequest;
    print("Payment Request: {}", paymentRequest);
    // ANCHOR_END: receive-payment
    return response
}
