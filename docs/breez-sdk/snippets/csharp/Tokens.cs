using System.Numerics;
using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class Tokens
    {
        async Task FetchTokenBalances(BreezSdk sdk)
        {
            // ANCHOR: fetch-token-balances
            // ensureSynced: true will ensure the SDK is synced with the Spark network
            // before returning the balance
            var info = await sdk.GetInfo(request: new GetInfoRequest(ensureSynced: false));

            // Token balances are a map of token identifier to balance
            var tokenBalances = info.tokenBalances;
            foreach (var kvp in tokenBalances)
            {
                var tokenId = kvp.Key;
                var tokenBalance = kvp.Value;
                Console.WriteLine($"Token ID: {tokenId}");
                Console.WriteLine($"Balance: {tokenBalance.balance}");
                Console.WriteLine($"Name: {tokenBalance.tokenMetadata.name}");
                Console.WriteLine($"Ticker: {tokenBalance.tokenMetadata.ticker}");
                Console.WriteLine($"Decimals: {tokenBalance.tokenMetadata.decimals}");
            }
            // ANCHOR_END: fetch-token-balances
        }

        async Task FetchTokenMetadata(BreezSdk sdk)
        {
            // ANCHOR: fetch-token-metadata
            var response = await sdk.GetTokensMetadata(
                request: new GetTokensMetadataRequest(
                    tokenIdentifiers: new List<string> { "<token identifier 1>", "<token identifier 2>" }
                )
            );

            var tokensMetadata = response.tokensMetadata;
            foreach (var tokenMetadata in tokensMetadata)
            {
                Console.WriteLine($"Token ID: {tokenMetadata.identifier}");
                Console.WriteLine($"Name: {tokenMetadata.name}");
                Console.WriteLine($"Ticker: {tokenMetadata.ticker}");
                Console.WriteLine($"Decimals: {tokenMetadata.decimals}");
                Console.WriteLine($"Max Supply: {tokenMetadata.maxSupply}");
                Console.WriteLine($"Is Freezable: {tokenMetadata.isFreezable}");
            }
            // ANCHOR_END: fetch-token-metadata
        }

        async Task ReceiveTokenPaymentSparkInvoice(BreezSdk sdk)
        {
            // ANCHOR: receive-token-payment-spark-invoice
            var tokenIdentifier = "<token identifier>";
            var optionalDescription = "<invoice description>";
            var optionalAmount = new BigInteger(5000);
            var optionalExpiryTimeSeconds = 1716691200UL;
            var optionalSenderPublicKey = "<sender public key>";

            var request = new ReceivePaymentRequest(
                paymentMethod: new ReceivePaymentMethod.SparkInvoice(
                    tokenIdentifier: tokenIdentifier,
                    description: optionalDescription,
                    amount: optionalAmount,
                    expiryTime: optionalExpiryTimeSeconds,
                    senderPublicKey: optionalSenderPublicKey
                )
            );
            var response = await sdk.ReceivePayment(request: request);

            var paymentRequest = response.paymentRequest;
            Console.WriteLine($"Payment request: {paymentRequest}");
            var receiveFee = response.fee;
            Console.WriteLine($"Fees: {receiveFee} token base units");
            // ANCHOR_END: receive-token-payment-spark-invoice
        }

        async Task SendTokenPayment(BreezSdk sdk)
        {
            // ANCHOR: send-token-payment
            var paymentRequest = "<spark address or invoice>";
            // Token identifier must match the invoice in case it specifies one.
            var tokenIdentifier = "<token identifier>";
            // Set the amount of tokens you wish to send.
            var optionalAmount = new BigInteger(1000);

            var prepareResponse = await sdk.PrepareSendPayment(
                request: new PrepareSendPaymentRequest(
                    paymentRequest: paymentRequest,
                    amount: optionalAmount,
                    tokenIdentifier: tokenIdentifier
                )
            );

            // If the fees are acceptable, continue to send the token payment
            if (prepareResponse.paymentMethod is SendPaymentMethod.SparkAddress sparkAddress)
            {
                Console.WriteLine($"Token ID: {sparkAddress.tokenIdentifier}");
                Console.WriteLine($"Fees: {sparkAddress.fee} token base units");
            }
            if (prepareResponse.paymentMethod is SendPaymentMethod.SparkInvoice sparkInvoice)
            {
                Console.WriteLine($"Token ID: {sparkInvoice.tokenIdentifier}");
                Console.WriteLine($"Fees: {sparkInvoice.fee} token base units");
            }

            // Send the token payment
            var sendResponse = await sdk.SendPayment(
                request: new SendPaymentRequest(
                    prepareResponse: prepareResponse,
                    options: null
                )
            );
            var payment = sendResponse.payment;
            Console.WriteLine($"Payment: {payment}");
            // ANCHOR_END: send-token-payment
        }

        async Task PrepareSendPaymentTokenConversion(BreezSdk sdk)
        {
            // ANCHOR: prepare-send-payment-token-conversion
            var paymentRequest = "<spark address or invoice>";
            // Token identifier must match the invoice in case it specifies one.
            var tokenIdentifier = "<token identifier>";
            // Set the amount of tokens you wish to send.
            var optionalAmount = new BigInteger(1000);
            // Optionally set to use Bitcoin funds to pay via token conversion
            var optionalMaxSlippageBps = 50U;
            var optionalCompletionTimeoutSecs = 30U;
            var tokenConversionOptions = new TokenConversionOptions(
                conversionType: new TokenConversionType.FromBitcoin(),
                maxSlippageBps: optionalMaxSlippageBps,
                completionTimeoutSecs: optionalCompletionTimeoutSecs
            );

            var prepareResponse = await sdk.PrepareSendPayment(
                request: new PrepareSendPaymentRequest(
                    paymentRequest: paymentRequest,
                    amount: optionalAmount,
                    tokenIdentifier: tokenIdentifier,
                    tokenConversionOptions: tokenConversionOptions
                )
            );

            // If the fees are acceptable, continue to send the token payment
            if (prepareResponse.tokenConversionFee != null)
            {
                Console.WriteLine("Estimated token conversion fee: " +
                    $"{prepareResponse.tokenConversionFee} sats");
            }
            // ANCHOR_END: prepare-send-payment-token-conversion
        }
    }
}
