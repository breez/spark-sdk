import BreezSdkSpark
import Foundation

func getPayment(sdk: BreezSdk) async throws -> Payment {
    // ANCHOR: get-payment
    let paymentId = "<payment id>"
    let response = try await sdk.getPayment(
        request: GetPaymentRequest(paymentId: paymentId)
    )
    let payment = response.payment
    // ANCHOR_END: get-payment
    return payment
}

func listPayments(sdk: BreezSdk) async throws -> [Payment] {
    // ANCHOR: list-payments
    let response = try await sdk.listPayments(
        request: ListPaymentsRequest(
            offset: nil,
            limit: nil
        ))
    let payments = response.payments
    // ANCHOR_END: list-payments
    return payments
}

func listPaymentsFiltered(sdk: BreezSdk) async throws -> [Payment] {
    // ANCHOR: list-payments-filtered
    let response = try await sdk.listPayments(
        request: ListPaymentsRequest(
            offset: 0,
            limit: 50
        ))
    let payments = response.payments
    // ANCHOR_END: list-payments-filtered
    return payments
}
