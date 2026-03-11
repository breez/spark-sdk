import BigNumber
import BreezSdkSpark

func prepareSendPaymentReserveLeaves(sdk: BreezSdk) async throws {
    // ANCHOR: prepare-send-payment-reserve-leaves
    let paymentRequest = "<payment request>"
    let amountSats: BInt? = BInt(50_000)

    let prepareResponse = try await sdk.prepareSendPayment(
        request: PrepareSendPaymentRequest(
            paymentRequest: paymentRequest,
            amount: amountSats,
            tokenIdentifier: nil,
            conversionOptions: nil,
            feePolicy: nil,
            reserveLeaves: true
        ))

    // The reservation ID can be used to cancel the reservation if needed
    if let reservationId = prepareResponse.reservationId {
        print("Reservation ID: \(reservationId)")
    }

    // Send payment as usual using the prepare response
    // try await sdk.sendPayment(request: SendPaymentRequest(prepareResponse: prepareResponse))
    // ANCHOR_END: prepare-send-payment-reserve-leaves
}

func cancelPrepareSendPayment(sdk: BreezSdk) async throws {
    // ANCHOR: cancel-prepare-send-payment
    let reservationId = "<reservation id from prepare response>"

    try await sdk.cancelPrepareSendPayment(
        request: CancelPrepareSendPaymentRequest(reservationId: reservationId))
    // ANCHOR_END: cancel-prepare-send-payment
}
