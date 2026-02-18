using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class Htlcs
    {
        async Task SendHtlcPayment(BreezSdk sdk)
        {
            // ANCHOR: send-htlc-payment
            var paymentRequest = "<spark address>";
            // Set the amount you wish the pay the receiver
            ulong? amountSats = 50_000UL;
            var prepareRequest = new PrepareSendPaymentRequest(
                paymentRequest: paymentRequest,
                amount: amountSats,
                tokenIdentifier: null,
                conversionOptions: null,
                feePolicy: null
            );
            var prepareResponse = await sdk.PrepareSendPayment(request: prepareRequest);

            // If the fees are acceptable, continue to create the HTLC Payment
            if (prepareResponse.paymentMethod is SendPaymentMethod.SparkAddress sparkMethod)
            {
                var fee = sparkMethod.fee;
                Console.WriteLine($"Fees: {fee} sats");
            }

            var preimage = "<32-byte unique preimage hex>";
            var preimageBytes = Convert.FromHexString(preimage);
            var paymentHashBytes = System.Security.Cryptography.SHA256.HashData(preimageBytes);
            var paymentHash = Convert.ToHexString(paymentHashBytes).ToLower();

            // Set the HTLC options
            var options = new SendPaymentOptions.SparkAddress(
                htlcOptions: new SparkHtlcOptions(
                    paymentHash: paymentHash,
                    expiryDurationSecs: 1000
                )
            );

            var request = new SendPaymentRequest(
                prepareResponse: prepareResponse,
                options: options
            );
            var sendResponse = await sdk.SendPayment(request: request);
            var payment = sendResponse.payment;
            // ANCHOR_END: send-htlc-payment
        }

        async Task ReceiveHodlInvoicePayment(BreezSdk sdk)
        {
            // ANCHOR: receive-hodl-invoice-payment
            var preimage = "<32-byte unique preimage hex>";
            var preimageBytes = Convert.FromHexString(preimage);
            var paymentHashBytes = System.Security.Cryptography.SHA256.HashData(preimageBytes);
            var paymentHash = Convert.ToHexString(paymentHashBytes).ToLower();

            var response = await sdk.ReceivePayment(
                request: new ReceivePaymentRequest(
                    paymentMethod: new ReceivePaymentMethod.Bolt11Invoice(
                        description: "HODL invoice",
                        amountSats: 50_000UL,
                        expirySecs: null,
                        paymentHash: paymentHash
                    )
                )
            );

            var invoice = response.paymentRequest;
            Console.WriteLine($"HODL invoice: {invoice}");
            // ANCHOR_END: receive-hodl-invoice-payment
        }

        async Task ListClaimableHtlcPayments(BreezSdk sdk)
        {
            // ANCHOR: list-claimable-htlc-payments
            var request = new ListPaymentsRequest(
                typeFilter: new List<PaymentType> { PaymentType.Receive },
                statusFilter: new List<PaymentStatus> { PaymentStatus.Pending },
                paymentDetailsFilter: new List<PaymentDetailsFilter> {
                    new PaymentDetailsFilter.Spark(
                        htlcStatus: new List<SparkHtlcStatus> {
                            SparkHtlcStatus.WaitingForPreimage
                        },
                        conversionRefundNeeded: null
                    ),
                    new PaymentDetailsFilter.Lightning(
                        htlcStatus: new List<SparkHtlcStatus> {
                            SparkHtlcStatus.WaitingForPreimage
                        }
                    )
                }
            );

            var response = await sdk.ListPayments(request: request);
            var payments = response.payments;

            foreach (var payment in payments)
            {
                if (payment.details is PaymentDetails.Spark sparkDetails && sparkDetails.htlcDetails != null)
                {
                    Console.WriteLine($"Spark HTLC expiry time: {sparkDetails.htlcDetails.expiryTime}");
                }
                else if (payment.details is PaymentDetails.Lightning lightningDetails)
                {
                    Console.WriteLine($"Lightning HTLC expiry time: {lightningDetails.htlcDetails.expiryTime}");
                }
            }
            // ANCHOR_END: list-claimable-htlc-payments
        }

        async Task ClaimHtlcPayment(BreezSdk sdk)
        {
            // ANCHOR: claim-htlc-payment
            var preimage = "<preimage hex>";
            var response = await sdk.ClaimHtlcPayment(
                request: new ClaimHtlcPaymentRequest(preimage: preimage)
            );
            var payment = response.payment;
            // ANCHOR_END: claim-htlc-payment
        }
    }
}
