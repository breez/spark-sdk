import BigNumber
import BreezSdkSpark

func prepareSendPaymentLightningBolt11(sdk: BreezSdk) async throws {
    // ANCHOR: prepare-send-payment-lightning-bolt11
    let paymentRequest = "<bolt11 invoice>"
    // Optionally set the amount you wish the pay the receiver (requires 'import BigNumber')
    let optionalAmountSats = BInt(5_000)

    let prepareResponse = try await sdk.prepareSendPayment(
        request: PrepareSendPaymentRequest(
            paymentRequest: paymentRequest,
            amount: optionalAmountSats
        ))

    if case let .bolt11Invoice(_, sparkTransferFeeSats, lightningFeeSats) = prepareResponse
        .paymentMethod
    {
        // Fees to pay via Lightning
        print("Lightning Fee: \(lightningFeeSats) sats")
        // Or fees to pay (if available) via a Spark transfer
        if let sparkTransferFeeSats = sparkTransferFeeSats {
            print("Spark Transfer Fee: \(sparkTransferFeeSats) sats")
        }
    }
    // ANCHOR_END: prepare-send-payment-lightning-bolt11
}

func prepareSendPaymentOnchain(sdk: BreezSdk) async throws {
    // ANCHOR: prepare-send-payment-onchain
    let paymentRequest = "<bitcoin address>"
    // Set the amount you wish the pay the receiver (requires 'import BigNumber')
    let amountSats = BInt(50_000)

    let prepareResponse = try await sdk.prepareSendPayment(
        request: PrepareSendPaymentRequest(
            paymentRequest: paymentRequest,
            amount: amountSats
        ))

    if case let .bitcoinAddress(_, feeQuote) = prepareResponse.paymentMethod {
        let slowFeeSats = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat
        let mediumFeeSats = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat
        let fastFeeSats = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat
        print("Slow Fees: \(slowFeeSats) sats")
        print("Medium Fees: \(mediumFeeSats) sats")
        print("Fast Fees: \(fastFeeSats) sats")
    }
    // ANCHOR_END: prepare-send-payment-onchain
}

func prepareSendPaymentSparkAddress(sdk: BreezSdk) async throws {
    // ANCHOR: prepare-send-payment-spark-address
    let paymentRequest = "<spark address>"
    // Set the amount you wish the pay the receiver (requires 'import BigNumber')
    let amountSats = BInt(50_000)

    let prepareResponse = try await sdk.prepareSendPayment(
        request: PrepareSendPaymentRequest(
            paymentRequest: paymentRequest,
            amount: amountSats
        ))

    if case let .sparkAddress(_, feeSats, _) = prepareResponse.paymentMethod {
        print("Fees: \(feeSats) sats")
    }
    // ANCHOR_END: prepare-send-payment-spark-address
}

func prepareSendPaymentSparkInvoice(sdk: BreezSdk) async throws {
    // ANCHOR: prepare-send-payment-spark-invoice
    let paymentRequest = "<spark invoice>"
    // Optionally set the amount you wish the pay the receiver (requires 'import BigNumber')
    let optionalAmountSats = BInt(50_000)

    let prepareResponse = try await sdk.prepareSendPayment(
        request: PrepareSendPaymentRequest(
            paymentRequest: paymentRequest,
            amount: optionalAmountSats
        ))

    if case let .sparkInvoice(_, feeSats, _) = prepareResponse.paymentMethod {
        print("Fees: \(feeSats) sats")
    }
    // ANCHOR_END: prepare-send-payment-spark-invoice
}

func prepareSendTokenPaymentTokenConversion(sdk: BreezSdk) async throws {
    // ANCHOR: prepare-send-payment-with-conversion
    let paymentRequest = "<bolt11 invoice>"
    // Set to use token funds to pay via conversion
    let optionalMaxSlippageBps = UInt32(50)
    let optionalCompletionTimeoutSecs = UInt32(30)
    let conversionOptions = ConversionOptions(
        conversionType: ConversionType.toBitcoin(
            fromTokenIdentifier: "<token identifier>"
        ),
        maxSlippageBps: optionalMaxSlippageBps,
        completionTimeoutSecs: optionalCompletionTimeoutSecs
    )

    let prepareResponse = try await sdk.prepareSendPayment(
        request: PrepareSendPaymentRequest(
            paymentRequest: paymentRequest,
            conversionOptions: conversionOptions
        ))

    if let conversionEstimate = prepareResponse.conversionEstimate {
        print("Estimated conversion amount: \(conversionEstimate.amount) token base units")
        print("Estimated conversion fee: \(conversionEstimate.fee) token base units")
    }
    // ANCHOR_END: prepare-send-payment-with-conversion
}

func sendPaymentLightningBolt11(sdk: BreezSdk, prepareResponse: PrepareSendPaymentResponse)
    async throws
{
    // ANCHOR: send-payment-lightning-bolt11
    let options = SendPaymentOptions.bolt11Invoice(preferSpark: false, completionTimeoutSecs: 10)
    let optionalIdempotencyKey = "<idempotency key uuid>"
    let sendResponse = try await sdk.sendPayment(
        request: SendPaymentRequest(
            prepareResponse: prepareResponse,
            options: options,
            idempotencyKey: optionalIdempotencyKey
        ))
    let payment = sendResponse.payment
    // ANCHOR_END: send-payment-lightning-bolt11
    print(payment)
}

func sendPaymentOnchain(sdk: BreezSdk, prepareResponse: PrepareSendPaymentResponse) async throws {
    // ANCHOR: send-payment-onchain
    let options = SendPaymentOptions.bitcoinAddress(
        confirmationSpeed: .medium
    )
    let optionalIdempotencyKey = "<idempotency key uuid>"
    let sendResponse = try await sdk.sendPayment(
        request: SendPaymentRequest(
            prepareResponse: prepareResponse,
            options: options,
            idempotencyKey: optionalIdempotencyKey
        ))
    let payment = sendResponse.payment
    // ANCHOR_END: send-payment-onchain
    print(payment)
}

func sendPaymentSpark(sdk: BreezSdk, prepareResponse: PrepareSendPaymentResponse) async throws {
    // ANCHOR: send-payment-spark
    let optionalIdempotencyKey = "<idempotency key uuid>"
    let sendResponse = try await sdk.sendPayment(
        request: SendPaymentRequest(
            prepareResponse: prepareResponse,
            idempotencyKey: optionalIdempotencyKey
        ))
    let payment = sendResponse.payment
    // ANCHOR_END: send-payment-spark
    print(payment)
}
