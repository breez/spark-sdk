using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class ReserveLeaves
    {
        async Task PrepareSendPaymentReserveLeaves(BreezSdk sdk)
        {
            // ANCHOR: prepare-send-payment-reserve-leaves
            var paymentRequest = "<payment request>";
            ulong? amountSats = 50_000UL;

            var request = new PrepareSendPaymentRequest(
                paymentRequest: paymentRequest,
                amount: amountSats,
                tokenIdentifier: null,
                conversionOptions: null,
                feePolicy: null,
                reserveLeaves: true
            );
            var prepareResponse = await sdk.PrepareSendPayment(request: request);

            // The reservation ID can be used to cancel the reservation if needed
            if (prepareResponse.reservationId != null)
            {
                Console.WriteLine($"Reservation ID: {prepareResponse.reservationId}");
            }

            // Send payment as usual using the prepare response
            // await sdk.SendPayment(new SendPaymentRequest(prepareResponse: prepareResponse));
            // ANCHOR_END: prepare-send-payment-reserve-leaves
        }

        async Task CancelPrepareSendPayment(BreezSdk sdk)
        {
            // ANCHOR: cancel-prepare-send-payment
            var reservationId = "<reservation id from prepare response>";

            await sdk.CancelPrepareSendPayment(
                request: new CancelPrepareSendPaymentRequest(reservationId: reservationId));
            // ANCHOR_END: cancel-prepare-send-payment
        }
    }
}
