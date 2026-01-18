using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class LnurlPay
    {
        async Task PreparePay(BreezSdk sdk)
        {
            // ANCHOR: prepare-lnurl-pay
            // Endpoint can also be of the form:
            // lnurlp://domain.com/lnurl-pay?key=val
            // lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43r
            //     vv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3k
            //     vdnxx5crxwpjvyunsephsz36jf
            var lnurlPayUrl = "lightning@address.com";
            var parsedInput = await sdk.Parse(lnurlPayUrl);
            if (parsedInput is InputType.LightningAddress lightningAddress)
            {
                var details = lightningAddress.v1;
                var amountSats = 5_000UL;
                var optionalComment = "<comment>";
                var payRequest = details.payRequest;
                var optionalValidateSuccessActionUrl = true;
                // Optionally set to use token funds to pay via token conversion
                var optionalMaxSlippageBps = 50U;
                var optionalCompletionTimeoutSecs = 30U;
                var optionalConversionOptions = new ConversionOptions(
                    conversionType: new ConversionType.ToBitcoin(
                        fromTokenIdentifier: "<token identifier>"
                    ),
                    maxSlippageBps: optionalMaxSlippageBps,
                    completionTimeoutSecs: optionalCompletionTimeoutSecs
                );

                var request = new PrepareLnurlPayRequest(
                    amountSats: amountSats,
                    payRequest: payRequest,
                    comment: optionalComment,
                    validateSuccessActionUrl: optionalValidateSuccessActionUrl,
                    conversionOptions: optionalConversionOptions
                );
                var prepareResponse = await sdk.PrepareLnurlPay(request: request);

                // If the fees are acceptable, continue to create the LNURL Pay
                if (prepareResponse.conversionEstimate != null)
                {
                    Console.WriteLine("Estimated conversion amount: " +
                        $"{prepareResponse.conversionEstimate.amount} token base units");
                    Console.WriteLine("Estimated conversion fee: " +
                        $"{prepareResponse.conversionEstimate.fee} token base units");
                }
                var feeSats = prepareResponse.feeSats;
                Console.WriteLine($"Fees: {feeSats} sats");
            }
            // ANCHOR_END: prepare-lnurl-pay
        }

        async Task Pay(BreezSdk sdk, PrepareLnurlPayResponse prepareResponse)
        {
            // ANCHOR: lnurl-pay
            var optionalIdempotencyKey = "<idempotency key uuid>";
            var response = await sdk.LnurlPay(
                new LnurlPayRequest(
                    prepareResponse: prepareResponse,
                    idempotencyKey: optionalIdempotencyKey
                )
            );
            // ANCHOR_END: lnurl-pay
        }
    }
}
