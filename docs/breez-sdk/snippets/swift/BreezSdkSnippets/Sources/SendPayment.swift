import BigInt
import BreezSdkSpark

func prepareSendPaymentLightningBolt11(sdk: BreezSdk) async throws {
    // ANCHOR: prepare-send-payment-lightning-bolt11
    let paymentRequest = "<bolt11 invoice>"
    // Optionally set the amount you wish the pay the receiver
    let optionalAmountSats: BigUInt = 5_000

    let prepareResponse = try await sdk.prepareSendPayment(
        request: PrepareSendPaymentRequest(
            paymentRequest: paymentRequest,
            amount: optionalAmountSats,
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
    // Set the amount you wish the pay the receiver
    let amountSats: BigUInt = 50_000

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

func prepareSendPaymentSpark(sdk: BreezSdk) async throws {
    // ANCHOR: prepare-send-payment-spark
    let paymentRequest = "<spark address>"
    // Set the amount you wish the pay the receiver
    let amountSats: BigUInt = 50_000

    let prepareResponse = try await sdk.prepareSendPayment(
        request: PrepareSendPaymentRequest(
            paymentRequest: paymentRequest,
            amount: amountSats
        ))

    if case let .sparkAddress(_, feeSats, _) = prepareResponse.paymentMethod {
        print("Fees: \(feeSats) sats")
    }
    // ANCHOR_END: prepare-send-payment-spark
}

func sendPaymentLightningBolt11(sdk: BreezSdk, prepareResponse: PrepareSendPaymentResponse)
    async throws
{
    // ANCHOR: send-payment-lightning-bolt11
    let options = SendPaymentOptions.bolt11Invoice(useSpark: true)
    let sendResponse = try await sdk.sendPayment(
        request: SendPaymentRequest(
            prepareResponse: prepareResponse,
            options: options
        ))
    let payment = sendResponse.payment
    // ANCHOR_END: send-payment-lightning-bolt11
    print(payment)
}

func sendPaymentOnchain(sdk: BreezSdk, prepareResponse: PrepareSendPaymentResponse) async throws {
    // ANCHOR: send-payment-onchain
    let options = SendPaymentOptions.bitcoinAddress(confirmationSpeed: .medium)
    let sendResponse = try await sdk.sendPayment(
        request: SendPaymentRequest(
            prepareResponse: prepareResponse,
            options: options
        ))
    let payment = sendResponse.payment
    // ANCHOR_END: send-payment-onchain
    print(payment)
}

func sendPaymentSpark(sdk: BreezSdk, prepareResponse: PrepareSendPaymentResponse) async throws {
    // ANCHOR: send-payment-spark
    let sendResponse = try await sdk.sendPayment(
        request: SendPaymentRequest(
            prepareResponse: prepareResponse,
            options: nil
        ))
    let payment = sendResponse.payment
    // ANCHOR_END: send-payment-spark
    print(payment)
}
