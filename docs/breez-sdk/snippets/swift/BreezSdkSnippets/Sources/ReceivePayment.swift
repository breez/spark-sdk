import BigNumber
import BreezSdkSpark

func receiveLightning(sdk: BreezSdk) async throws -> ReceivePaymentResponse {
    // ANCHOR: receive-payment-lightning-bolt11
    let description = "<invoice description>"
    // Optionally set the invoice amount you wish the payer to send
    let optionalAmountSats: UInt64 = 5_000
    // Optionally set the expiry duration in seconds
    let optionalExpirySecs: UInt32 = 3600
    let optionalReceiverIdentityPublicKey: String? = nil
    // Set to FeePolicy.feesIncluded to deduct a partner fee from the amount
    // instead of adding it on top for the payer
    let optionalFeePolicy: FeePolicy? = nil
    let response =
        try await sdk
        .receivePayment(
            request: ReceivePaymentRequest(
                paymentMethod: ReceivePaymentMethod.bolt11Invoice(
                    description: description,
                    amountSats: optionalAmountSats,
                    expirySecs: optionalExpirySecs,
                    paymentHash: nil,
                    receiverIdentityPublicKey: optionalReceiverIdentityPublicKey,
                    feePolicy: optionalFeePolicy
                )
            ))

    let paymentRequest = response.paymentRequest
    print("Payment Request: {}", paymentRequest)
    switch response.fee {
    case let .fixed(amount):
        print("Fees: {} sats", amount)
    // Only on an amountless invoice: a share of what the payer sends
    case let .rate(bps, fixedSats):
        print("Fees: {} bps of the amount sent plus {} sats", bps, fixedSats)
    }
    // ANCHOR_END: receive-payment-lightning-bolt11

    return response
}

func receiveOnchain(sdk: BreezSdk) async throws -> ReceivePaymentResponse {
    // ANCHOR: receive-payment-onchain
    let newAddress: Bool? = nil // Set to true to get a new address
    let response =
        try await sdk
        .receivePayment(
            request: ReceivePaymentRequest(
                paymentMethod: ReceivePaymentMethod.bitcoinAddress(
                    newAddress: newAddress)
            ))

    let paymentRequest = response.paymentRequest
    print("Payment Request: {}", paymentRequest)
    if case let .fixed(amount) = response.fee {
        print("Fees: {} sats", amount)
    }
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
    if case let .fixed(amount) = response.fee {
        print("Fees: {} sats", amount)
    }
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
    if case let .fixed(amount) = response.fee {
        print("Fees: {} sats", amount)
    }
    // ANCHOR_END: receive-payment-spark-invoice

    return response
}
