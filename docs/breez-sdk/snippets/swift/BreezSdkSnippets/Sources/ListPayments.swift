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
        request: ListPaymentsRequest())
    let payments = response.payments
    // ANCHOR_END: list-payments
    return payments
}

func listPaymentsFiltered(sdk: BreezSdk) async throws -> [Payment] {
    // ANCHOR: list-payments-filtered
    // Filter by asset (Bitcoin or Token)
    let assetFilter = AssetFilter.token(tokenIdentifier: "token_identifier_here")
    // To filter by Bitcoin instead:
    // let assetFilter = AssetFilter.bitcoin

    let response = try await sdk.listPayments(
        request: ListPaymentsRequest(
            // Filter by payment type
            typeFilter: [PaymentType.send, PaymentType.receive],
            // Filter by status
            statusFilter: [PaymentStatus.completed],
            assetFilter: assetFilter,
            // Time range filters
            fromTimestamp: 1_704_067_200,  // Unix timestamp
            toTimestamp: 1_735_689_600,  // Unix timestamp
            // Pagination
            offset: 0,
            limit: 50,
            // Sort order (true = oldest first, false = newest first)
            sortAscending: false
        ))
    let payments = response.payments
    // ANCHOR_END: list-payments-filtered
    return payments
}
