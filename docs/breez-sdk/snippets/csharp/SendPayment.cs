using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class SendPayment
    {
        async Task PrepareSendPaymentLightningBolt11(BreezSdk sdk)
        {
            // ANCHOR: prepare-send-payment-lightning-bolt11
            var paymentRequest = "<bolt11 invoice>";
            // Optionally set the amount you wish to pay the receiver
            ulong? optionalAmountSats = 5_000UL;

            var request = new PrepareSendPaymentRequest(
                paymentRequest: paymentRequest,
                amount: optionalAmountSats,
                tokenIdentifier: null,
                conversionOptions: null,
                feePolicy: null
            );
            var prepareResponse = await sdk.PrepareSendPayment(request: request);

            // If the fees are acceptable, continue to create the Send Payment
            if (prepareResponse.paymentMethod is SendPaymentMethod.Bolt11Invoice bolt11Method)
            {
                // Fees to pay via Lightning
                var lightningFeeSats = bolt11Method.lightningFeeSats;
                // Or fees to pay (if available) via a Spark transfer
                var sparkTransferFeeSats = bolt11Method.sparkTransferFeeSats;
                Console.WriteLine($"Lightning Fees: {lightningFeeSats} sats");
                Console.WriteLine($"Spark Transfer Fees: {sparkTransferFeeSats} sats");
            }
            // ANCHOR_END: prepare-send-payment-lightning-bolt11
        }

        async Task PrepareSendPaymentOnchain(BreezSdk sdk)
        {
            // ANCHOR: prepare-send-payment-onchain
            var paymentRequest = "<bitcoin address>";
            // Set the amount you wish to pay the receiver
            ulong? amountSats = 50_000UL;

            var request = new PrepareSendPaymentRequest(
                paymentRequest: paymentRequest,
                amount: amountSats,
                tokenIdentifier: null,
                conversionOptions: null,
                feePolicy: null
            );
            var prepareResponse = await sdk.PrepareSendPayment(request: request);

            // Review the fee quote for each confirmation speed
            if (prepareResponse.paymentMethod is SendPaymentMethod.BitcoinAddress bitcoinMethod)
            {
                var feeQuote = bitcoinMethod.feeQuote;
                var slowFeeSats = feeQuote.speedSlow.userFeeSat + feeQuote.speedSlow.l1BroadcastFeeSat;
                var mediumFeeSats = feeQuote.speedMedium.userFeeSat + feeQuote.speedMedium.l1BroadcastFeeSat;
                var fastFeeSats = feeQuote.speedFast.userFeeSat + feeQuote.speedFast.l1BroadcastFeeSat;
                Console.WriteLine($"Slow fee: {slowFeeSats} sats");
                Console.WriteLine($"Medium fee: {mediumFeeSats} sats");
                Console.WriteLine($"Fast fee: {fastFeeSats} sats");
            }
            // ANCHOR_END: prepare-send-payment-onchain
        }

        async Task PrepareSendPaymentSparkAddress(BreezSdk sdk)
        {
            // ANCHOR: prepare-send-payment-spark-address
            var paymentRequest = "<spark address>";
            // Set the amount you wish to pay the receiver
            ulong? amountSats = 50_000UL;

            var request = new PrepareSendPaymentRequest(
                paymentRequest: paymentRequest,
                amount: amountSats,
                tokenIdentifier: null,
                conversionOptions: null,
                feePolicy: null
            );
            var prepareResponse = await sdk.PrepareSendPayment(request: request);

            // If the fees are acceptable, continue to create the Send Payment
            if (prepareResponse.paymentMethod is SendPaymentMethod.SparkAddress sparkMethod)
            {
                var fee = sparkMethod.fee;
                Console.WriteLine($"Fees: {fee} sats");
            }
            // ANCHOR_END: prepare-send-payment-spark-address
        }

        async Task PrepareSendPaymentSparkInvoice(BreezSdk sdk)
        {
            // ANCHOR: prepare-send-payment-spark-invoice
            var paymentRequest = "<spark invoice>";
            // Optionally set the amount you wish to pay the receiver
            ulong? optionalAmountSats = 50_000UL;

            var request = new PrepareSendPaymentRequest(
                paymentRequest: paymentRequest,
                amount: optionalAmountSats,
                tokenIdentifier: null,
                conversionOptions: null,
                feePolicy: null
            );
            var prepareResponse = await sdk.PrepareSendPayment(request: request);

            // If the fees are acceptable, continue to create the Send Payment
            if (prepareResponse.paymentMethod is SendPaymentMethod.SparkInvoice sparkInvoiceMethod)
            {
                var fee = sparkInvoiceMethod.fee;
                Console.WriteLine($"Fees: {fee} sats");
            }
            // ANCHOR_END: prepare-send-payment-spark-invoice
        }

        async Task PrepareSendPaymentFeesIncluded(BreezSdk sdk)
        {
            // ANCHOR: prepare-send-payment-fees-included
            // By default (FeePolicy.FeesExcluded), fees are added on top of the amount.
            // Use FeePolicy.FeesIncluded to deduct fees from the amount instead.
            // The receiver gets amount minus fees.
            var paymentRequest = "<payment request>";
            ulong? amountSats = 50_000UL;

            var request = new PrepareSendPaymentRequest(
                paymentRequest: paymentRequest,
                amount: amountSats,
                tokenIdentifier: null,
                conversionOptions: null,
                feePolicy: FeePolicy.FeesIncluded
            );
            var prepareResponse = await sdk.PrepareSendPayment(request: request);

            // The response shows the fee policy used
            Console.WriteLine($"Fee policy: {prepareResponse.feePolicy}");
            Console.WriteLine($"Amount: {prepareResponse.amount}");
            // The receiver gets amount - fees (fees are available in prepareResponse.paymentMethod)
            // ANCHOR_END: prepare-send-payment-fees-included
        }

        async Task PrepareSendPaymentTokenConversion(BreezSdk sdk)
        {
            // ANCHOR: prepare-send-payment-with-conversion
            var paymentRequest = "<payment request>";
            // Set to use token funds to pay via conversion
            var optionalMaxSlippageBps = 50U;
            var optionalCompletionTimeoutSecs = 30U;
            var conversionOptions = new ConversionOptions(
                conversionType: new ConversionType.ToBitcoin(
                    fromTokenIdentifier: "<token identifier>"
                ),
                maxSlippageBps: optionalMaxSlippageBps,
                completionTimeoutSecs: optionalCompletionTimeoutSecs
            );

            var request = new PrepareSendPaymentRequest(
                paymentRequest: paymentRequest,
                amount: null,
                tokenIdentifier: null,
                conversionOptions: conversionOptions,
                feePolicy: null
            );
            var prepareResponse = await sdk.PrepareSendPayment(request: request);

            // If the fees are acceptable, continue to create the Send Payment
            if (prepareResponse.conversionEstimate != null)
            {
                Console.WriteLine("Estimated conversion amount: " +
                    $"{prepareResponse.conversionEstimate.amount} token base units");
                Console.WriteLine("Estimated conversion fee: " +
                    $"{prepareResponse.conversionEstimate.fee} token base units");
            }
            // ANCHOR_END: prepare-send-payment-with-conversion
        }

        async Task SendPaymentLightningBolt11(BreezSdk sdk, PrepareSendPaymentResponse prepareResponse)
        {
            // ANCHOR: send-payment-lightning-bolt11
            var options = new SendPaymentOptions.Bolt11Invoice(
                preferSpark: false,
                completionTimeoutSecs: 10
            );
            var optionalIdempotencyKey = "<idempotency key uuid>";
            var request = new SendPaymentRequest(
                prepareResponse: prepareResponse,
                options: options,
                idempotencyKey: optionalIdempotencyKey
            );
            var sendResponse = await sdk.SendPayment(request: request);
            var payment = sendResponse.payment;
            // ANCHOR_END: send-payment-lightning-bolt11
        }

        async Task SendPaymentOnchain(BreezSdk sdk, PrepareSendPaymentResponse prepareResponse)
        {
            // ANCHOR: send-payment-onchain
            // Select the confirmation speed for the on-chain transaction
            var options = new SendPaymentOptions.BitcoinAddress(
                confirmationSpeed: OnchainConfirmationSpeed.Medium
            );
            var optionalIdempotencyKey = "<idempotency key uuid>";
            var request = new SendPaymentRequest(
                prepareResponse: prepareResponse,
                options: options,
                idempotencyKey: optionalIdempotencyKey
            );
            var sendResponse = await sdk.SendPayment(request: request);
            var payment = sendResponse.payment;
            // ANCHOR_END: send-payment-onchain
        }

        async Task SendPaymentSpark(BreezSdk sdk, PrepareSendPaymentResponse prepareResponse)
        {
            // ANCHOR: send-payment-spark
            var optionalIdempotencyKey = "<idempotency key uuid>";
            var request = new SendPaymentRequest(
                prepareResponse: prepareResponse,
                idempotencyKey: optionalIdempotencyKey
            );
            var sendResponse = await sdk.SendPayment(request: request);
            var payment = sendResponse.payment;
            // ANCHOR_END: send-payment-spark
        }
    }
}
