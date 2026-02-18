import BigNumber
import BreezSdkSpark

func receiveLightning(sdk: BreezSdk) async throws -> ReceivePaymentResponse {
    // ANCHOR: receive-payment-lightning-bolt11
    let description = "<invoice description>"
    // Optionally set the invoice amount you wish the payer to send
    let optionalAmountSats: UInt64 = 5_000
    // Optionally set the expiry duration in seconds
    let optionalExpirySecs: UInt32 = 3600
    let response =
        try await sdk
        .receivePayment(
            request: ReceivePaymentRequest(
                paymentMethod: ReceivePaymentMethod.bolt11Invoice(
                    description: description,
                    amountSats: optionalAmountSats,
                    expirySecs: optionalExpirySecs,
                    paymentHash: nil
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
    // Optionally set the expiry UNIX timestamp in seconds
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
                    senderPublicKey: optionalSenderPublicKey
                )
            ))

    let paymentRequest = response.paymentRequest
    print("Payment Request: {}", paymentRequest)
    let receiveFeeSats = response.fee
    print("Fees: {} sats", receiveFeeSats)
    // ANCHOR_END: receive-payment-spark-invoice

    return response
}
