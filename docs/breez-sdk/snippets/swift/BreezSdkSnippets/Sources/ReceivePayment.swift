import BigNumber
import BreezSdkSpark

func receiveLightning(sdk: BreezSdk) async throws -> ReceivePaymentResponse {
    // ANCHOR: receive-payment-lightning-bolt11
    let description = "<invoice description>"
    // Optionally set the invoice amount you wish the payer to send
    let optionalAmountSats: UInt64 = 5_000
    let response =
        try await sdk
        .receivePayment(
            request: ReceivePaymentRequest(
                paymentMethod: ReceivePaymentMethod.bolt11Invoice(
                    description: description,
                    amountSats: optionalAmountSats
                )
            ))

    let paymentRequest = response.paymentRequest
    print("Payment Request: {}", paymentRequest)
    let receiveFeeSats = response.fee
    print("Fees: {} sats", receiveFeeSats)
    // ANCHOR_END: receive-payment-lightning-bolt11

    return response
}

func receiveOnchain(sdk: BreezSdk) async throws -> ReceivePaymentResponse {
    // ANCHOR: receive-payment-onchain
    let response =
        try await sdk
        .receivePayment(
            request: ReceivePaymentRequest(
                paymentMethod: ReceivePaymentMethod.bitcoinAddress
            ))

    let paymentRequest = response.paymentRequest
    print("Payment Request: {}", paymentRequest)
    let receiveFeeSats = response.fee
    print("Fees: {} sats", receiveFeeSats)
    // ANCHOR_END: receive-payment-onchain

    return response
}

func receiveSparkAddress(sdk: BreezSdk) async throws -> ReceivePaymentResponse {
    // ANCHOR: receive-payment-spark-address
    let response =
        try await sdk
        .receivePayment(
            request: ReceivePaymentRequest(
                paymentMethod: ReceivePaymentMethod.sparkAddress
            ))

    let paymentRequest = response.paymentRequest
    print("Payment Request: {}", paymentRequest)
    let receiveFeeSats = response.fee
    print("Fees: {} sats", receiveFeeSats)
    // ANCHOR_END: receive-payment-spark-address

    return response
}

func receiveSparkInvoice(sdk: BreezSdk) async throws -> ReceivePaymentResponse {
    // ANCHOR: receive-payment-spark-invoice
    let optionalDescription = "<invoice description>"
    let optionalAmountSats = BInt(5_000)
    let optionalExpiryTimeSeconds: UInt64 = 1_716_691_200
    let optionalSenderPublicKey = "<sender public key>"

    let response =
        try await sdk
        .receivePayment(
            request: ReceivePaymentRequest(
                paymentMethod: ReceivePaymentMethod.sparkInvoice(
                    amount: optionalAmountSats,
                    tokenIdentifier: nil,
                    expiryTime: optionalExpiryTimeSeconds,
                    description: optionalDescription,
                    senderPublicKey: optionalSenderPublicKey,
                )
            ))

    let paymentRequest = response.paymentRequest
    print("Payment Request: {}", paymentRequest)
    let receiveFeeSats = response.fee
    print("Fees: {} sats", receiveFeeSats)
    // ANCHOR_END: receive-payment-spark-invoice

    return response
}

func waitForPayment(sdk: BreezSdk) async throws {
    // ANCHOR: wait-for-payment
    // Waiting for a payment given its payment request (Bolt11 or Spark invoice)
    let paymentRequest = "<Bolt11 or Spark invoice>"

    // Wait for a payment to be completed using a payment request
    let paymentRequestResponse = try await sdk.waitForPayment(
        request: WaitForPaymentRequest(
            identifier: WaitForPaymentIdentifier.paymentRequest(paymentRequest)
        )
    )

    print("Payment received with ID: \(paymentRequestResponse.payment.id)")

    // Waiting for a payment given its payment id
    let paymentId = "<payment id>"

    // Wait for a payment to be completed using a payment id
    let paymentIdResponse = try await sdk.waitForPayment(
        request: WaitForPaymentRequest(
            identifier: WaitForPaymentIdentifier.paymentId(paymentId)
        )
    )

    print("Payment received with ID: \(paymentIdResponse.payment.id)")
    // ANCHOR_END: wait-for-payment
}
