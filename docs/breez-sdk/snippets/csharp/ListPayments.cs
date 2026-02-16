using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class ListPayments
    {
        async Task GetPayment(BreezClient client)
        {
            // ANCHOR: get-payment
            var paymentId = "<payment id>";
            var response = await client.GetPayment(
                request: new GetPaymentRequest(paymentId: paymentId)
            );
            var payment = response.payment;
            // ANCHOR_END: get-payment
        }

        async Task ListAllPayments(BreezClient client)
        {
            // ANCHOR: list-payments
            var response = await client.ListPayments(request: new ListPaymentsRequest());
            var payments = response.payments;
            // ANCHOR_END: list-payments
        }

        async Task ListPaymentsFiltered(BreezClient client)
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

            var response = await client.ListPayments(request: request);
            var payments = response.payments;
            // ANCHOR_END: list-payments-filtered
        }
    }
}
