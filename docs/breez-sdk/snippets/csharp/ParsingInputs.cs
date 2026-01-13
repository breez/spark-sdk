using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class ParsingInputs
    {
        async Task ParseInput(BreezSdk sdk)
        {
            // ANCHOR: parse-inputs
            var inputStr = "an input to be parsed...";

            var parsedInput = await sdk.Parse(input: inputStr);
            switch (parsedInput)
            {
                case InputType.BitcoinAddress bitcoinAddress:
                    var details = bitcoinAddress.v1;
                    Console.WriteLine($"Input is Bitcoin address {details.address}");
                    break;

                case InputType.Bolt11Invoice bolt11:
                    var bolt11Details = bolt11.v1;
                    var amount = bolt11Details.amountMsat.HasValue ? bolt11Details.amountMsat.Value.ToString() : "unknown";
                    Console.WriteLine($"Input is BOLT11 invoice for {amount} msats");
                    break;

                case InputType.LnurlPay lnurlPay:
                    var lnurlPayDetails = lnurlPay.v1;
                    Console.WriteLine($"Input is LNURL-Pay/Lightning address accepting " +
                                    $"min/max {lnurlPayDetails.minSendable}/{lnurlPayDetails.maxSendable} msats");
                    break;

                case InputType.LnurlWithdraw lnurlWithdraw:
                    var lnurlWithdrawDetails = lnurlWithdraw.v1;
                    Console.WriteLine($"Input is LNURL-Withdraw for min/max " +
                                    $"{lnurlWithdrawDetails.minWithdrawable}/{lnurlWithdrawDetails.maxWithdrawable} msats");
                    break;

                case InputType.SparkAddress sparkAddress:
                    var sparkAddressDetails = sparkAddress.v1;
                    Console.WriteLine($"Input is Spark address {sparkAddressDetails.address}");
                    break;

                case InputType.SparkInvoice sparkInvoice:
                    var invoice = sparkInvoice.v1;
                    Console.WriteLine("Input is Spark invoice:");
                    if (invoice.tokenIdentifier != null)
                    {
                        Console.WriteLine($"  Amount: {invoice.amount} base units of " +
                                        $"token with id {invoice.tokenIdentifier}");
                    }
                    else
                    {
                        Console.WriteLine($"  Amount: {invoice.amount} sats");
                    }

                    if (invoice.description != null)
                    {
                        Console.WriteLine($"  Description: {invoice.description}");
                    }

                    if (invoice.expiresAt.HasValue)
                    {
                        Console.WriteLine($"  Expires at: {invoice.expiresAt}");
                    }

                    if (invoice.senderPublicKey != null)
                    {
                        Console.WriteLine($"  Sender public key: {invoice.senderPublicKey}");
                    }
                    break;

                    // Other input types are available
            }
            // ANCHOR_END: parse-inputs
        }

        void SetExternalInputParsers()
        {
            // ANCHOR: set-external-input-parsers
            // Create the default config
            var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
            {
                apiKey = "<breez api key>",
                externalInputParsers = new List<ExternalInputParser>
                {
                new ExternalInputParser(
                    providerId: "provider_a",
                    inputRegex: "^provider_a",
                    parserUrl: "https://parser-domain.com/parser?input=<input>"
                ),
                new ExternalInputParser(
                    providerId: "provider_b",
                    inputRegex: "^provider_b",
                    parserUrl: "https://parser-domain.com/parser?input=<input>"
                )
                }
            };
            // ANCHOR_END: set-external-input-parsers
        }
    }
}
