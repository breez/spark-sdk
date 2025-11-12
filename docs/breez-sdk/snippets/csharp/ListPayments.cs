using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class ListPayments
    {
        async Task GetPayment(BreezSdk sdk)
        {
            // ANCHOR: get-payment
            var paymentId = "<payment id>";
            var response = await sdk.GetPayment(
                request: new GetPaymentRequest(paymentId: paymentId)
            );
            var payment = response.payment;
            // ANCHOR_END: get-payment
        }

        async Task ListAllPayments(BreezSdk sdk)
        {
            // ANCHOR: list-payments
            var response = await sdk.ListPayments(request: new ListPaymentsRequest());
            var payments = response.payments;
            // ANCHOR_END: list-payments
        }

        async Task ListPaymentsFiltered(BreezSdk sdk)
        {
            // ANCHOR: list-payments-filtered
            // Filter by asset (Bitcoin or Token)
            var assetFilter = new AssetFilter.Token(tokenIdentifier: "token_identifier_here");
            // To filter by Bitcoin instead:
            // var assetFilter = new AssetFilter.Bitcoin();

            var request = new ListPaymentsRequest(
                // Filter by payment type
                typeFilter: new List<PaymentType> { PaymentType.Send, PaymentType.Receive },
                // Filter by status
                statusFilter: new List<PaymentStatus> { PaymentStatus.Completed },
                assetFilter: assetFilter,
                // Time range filters
                fromTimestamp: 1704067200, // Unix timestamp
                toTimestamp: 1735689600,   // Unix timestamp
                                           // Pagination
                offset: 0,
                limit: 50,
                // Sort order (true = oldest first, false = newest first)
                sortAscending: false
            );

            var response = await sdk.ListPayments(request: request);
            var payments = response.payments;
            // ANCHOR_END: list-payments-filtered
        }
    }
}
