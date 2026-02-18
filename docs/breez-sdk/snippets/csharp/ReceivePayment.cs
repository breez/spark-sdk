using System.Numerics;
using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class ReceivePayment
    {
        async Task ReceiveLightning(BreezSdk sdk)
        {
            // ANCHOR: receive-payment-lightning-bolt11
            var description = "<invoice description>";
            // Optionally set the invoice amount you wish the payer to send
            var optionalAmountSats = 5_000UL;
            // Optionally set the expiry duration in seconds
            var optionalExpirySecs = 3600U;
            var paymentMethod = new ReceivePaymentMethod.Bolt11Invoice(
                description: description,
                amountSats: optionalAmountSats,
                expirySecs: optionalExpirySecs,
                paymentHash: null
            );
            var request = new ReceivePaymentRequest(paymentMethod: paymentMethod);
            var response = await sdk.ReceivePayment(request: request);

            var paymentRequest = response.paymentRequest;
            Console.WriteLine($"Payment Request: {paymentRequest}");
            var receiveFeeSats = response.fee;
            Console.WriteLine($"Fees: {receiveFeeSats} sats");
            // ANCHOR_END: receive-payment-lightning-bolt11
        }

        async Task ReceiveOnchain(BreezSdk sdk)
        {
            // ANCHOR: receive-payment-onchain
            var request = new ReceivePaymentRequest(
                paymentMethod: new ReceivePaymentMethod.BitcoinAddress()
            );
            var response = await sdk.ReceivePayment(request: request);

            var paymentRequest = response.paymentRequest;
            Console.WriteLine($"Payment Request: {paymentRequest}");
            var receiveFeeSats = response.fee;
            Console.WriteLine($"Fees: {receiveFeeSats} sats");
            // ANCHOR_END: receive-payment-onchain
        }

        async Task ReceiveSparkAddress(BreezSdk sdk)
        {
            // ANCHOR: receive-payment-spark-address
            var request = new ReceivePaymentRequest(
                paymentMethod: new ReceivePaymentMethod.SparkAddress()
            );
            var response = await sdk.ReceivePayment(request: request);

            var paymentRequest = response.paymentRequest;
            Console.WriteLine($"Payment Request: {paymentRequest}");
            var receiveFeeSats = response.fee;
            Console.WriteLine($"Fees: {receiveFeeSats} sats");
            // ANCHOR_END: receive-payment-spark-address
        }

        async Task ReceiveSparkInvoice(BreezSdk sdk)
        {
            // ANCHOR: receive-payment-spark-invoice
            var optionalDescription = "<invoice description>";
            var optionalAmountSats = new BigInteger(5000);
            // Optionally set the expiry UNIX timestamp in seconds
            var optionalExpiryTimeSeconds = 1716691200UL;
            var optionalSenderPublicKey = "<sender public key>";

            var request = new ReceivePaymentRequest(
                paymentMethod: new ReceivePaymentMethod.SparkInvoice(
                    description: optionalDescription,
                    amount: optionalAmountSats,
                    expiryTime: optionalExpiryTimeSeconds,
                    senderPublicKey: optionalSenderPublicKey,
                    tokenIdentifier: null
                )
            );
            var response = await sdk.ReceivePayment(request: request);

            var paymentRequest = response.paymentRequest;
            Console.WriteLine($"Payment Request: {paymentRequest}");
            var receiveFeeSats = response.fee;
            Console.WriteLine($"Fees: {receiveFeeSats} sats");
            // ANCHOR_END: receive-payment-spark-invoice
        }
    }
}
