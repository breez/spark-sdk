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

                var request = new PrepareLnurlPayRequest(
                    amountSats: amountSats,
                    payRequest: payRequest,
                    comment: optionalComment,
                    validateSuccessActionUrl: optionalValidateSuccessActionUrl
                );
                var prepareResponse = await sdk.PrepareLnurlPay(request: request);

                // If the fees are acceptable, continue to create the LNURL Pay
                var feeSats = prepareResponse.feeSats;
                Console.WriteLine($"Fees: {feeSats} sats");
            }
            // ANCHOR_END: prepare-lnurl-pay
        }

        async Task Pay(BreezSdk sdk, PrepareLnurlPayResponse prepareResponse)
        {
            // ANCHOR: lnurl-pay
            var response = await sdk.LnurlPay(
                new LnurlPayRequest(prepareResponse: prepareResponse)
            );
            // ANCHOR_END: lnurl-pay
        }
    }
}
