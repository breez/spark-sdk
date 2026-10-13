using System.Numerics;
using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class BuyingBitcoin
    {
        async Task BuyBitcoin(BreezSdk sdk)
        {
            // ANCHOR: buy-bitcoin
            // Optionally, prefill the purchase amount
            ulong? optionalAmountSat = 100_000;
            // Optionally, set a redirect URL for after the purchase is completed
            var optionalRedirectUrl = "https://example.com/purchase-complete";

            var request = new BuyBitcoinRequest.Moonpay(
                delivery: new MoonpayDelivery.Bitcoin(amountSat: optionalAmountSat),
                redirectUrl: optionalRedirectUrl
            );

            var response = await sdk.BuyBitcoin(request: request);
            Console.WriteLine("Open this URL in a browser to complete the purchase:");
            Console.WriteLine($"{response.url}");
            // ANCHOR_END: buy-bitcoin
        }

        async Task BuyBitcoinViaCrossChain(BreezSdk sdk)
        {
            // ANCHOR: buy-bitcoin-cross-chain
            // USD amount to receive, in 6-decimal base units ($50)
            var amount = new BigInteger(50_000_000);

            var request = new BuyBitcoinRequest.Moonpay(
                delivery: new MoonpayDelivery.CrossChain(
                    amount: amount,
                    feeMode: null
                ),
                redirectUrl: null
            );

            var response = await sdk.BuyBitcoin(request: request);
            Console.WriteLine("Open this URL in a browser to complete the purchase:");
            Console.WriteLine($"{response.url}");

            if (response.crossChainInfo is { } info)
            {
                Console.WriteLine($"USDC to buy: {info.depositAmount}");
                Console.WriteLine(
                    "Expected to receive: "
                        + $"{info.expectedReceivedAmount} {info.destinationAsset}"
                );
                Console.WriteLine($"Conversion fee: {info.serviceFeeAmount}");
            }
            // ANCHOR_END: buy-bitcoin-cross-chain
        }

        async Task BuyBitcoinViaCashapp(BreezSdk sdk)
        {
            // ANCHOR: buy-bitcoin-cashapp
            // Cash App requires the amount to be specified up front.
            var amountSats = (ulong)50_000;

            var request = new BuyBitcoinRequest.CashApp(
                amountSats: amountSats
            );

            var response = await sdk.BuyBitcoin(request: request);
            Console.WriteLine("Open this URL in Cash App to complete the purchase:");
            Console.WriteLine($"{response.url}");
            // ANCHOR_END: buy-bitcoin-cashapp
        }
    }
}
