using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class LnurlWithdraw
    {
        async Task Withdraw(BreezSdk sdk)
        {
            // ANCHOR: lnurl-withdraw
            // Endpoint can also be of the form:
            // lnurlw://domain.com/lnurl-withdraw?key=val
            var lnurlWithdrawUrl = "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4exctthd96xserjv9mn7um9wdekj" +
                                    "mmw843xxwpexdnxzen9vgunsvfexq6rvdecx93rgdmyxcuxverrvcursenpxvukzv3c8" +
                                    "qunsdecx33nzwpnvg6ryc3hv93nzvecxgcxgwp3h33lxk";

            var inputType = await sdk.Parse(lnurlWithdrawUrl);
            if (inputType is InputType.LnurlWithdraw lnurlWithdraw)
            {
                // Amount to withdraw in sats between min/max withdrawable amounts
                var amountSats = 5_000UL;
                var withdrawRequest = lnurlWithdraw.v1;
                var optionalCompletionTimeoutSecs = 30U;

                var request = new LnurlWithdrawRequest(
                    amountSats: amountSats,
                    withdrawRequest: withdrawRequest,
                    completionTimeoutSecs: optionalCompletionTimeoutSecs
                );
                var response = await sdk.LnurlWithdraw(request: request);

                var payment = response.payment;
                Console.WriteLine($"Payment: {payment}");
            }
            // ANCHOR_END: lnurl-withdraw
        }
    }
}
