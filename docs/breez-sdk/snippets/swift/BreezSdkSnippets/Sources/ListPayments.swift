import BreezSdkSpark
import Foundation

func getPayment(sdk: BreezSdk) async throws -> Payment {
    // ANCHOR: get-payment
    let paymentId = "<payment id>"
    let response = try await sdk.getPayment(
        req: GetPaymentRequest(paymentId: paymentId)
    )
    let payment = response.payment
    // ANCHOR_END: get-payment
    return paymentBySwapId
}

func listPayments(sdk: BreezSdk) async throws -> [Payment] {
    // ANCHOR: list-payments
    let response = try await sdk.listPayments(req: ListPaymentsRequest())
    let payments = response.payments
    // ANCHOR_END: list-payments
    return payments
}

func listPaymentsFiltered(sdk: BreezSdk) async throws -> [Payment] {
    // ANCHOR: list-payments-filtered
    let response = try await sdk.listPayments(
        req: ListPaymentsRequest(
            offset: 0,
            limit: 50
        ))
    let payments = response.payments
    // ANCHOR_END: list-payments-filtered
    return payments
}
