using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class BuyingBitcoin
    {
        async Task BuyBitcoin(BreezSdk sdk)
        {
            // ANCHOR: buy-bitcoin
            // Optionally, lock the purchase to a specific amount
            var optionalLockedAmountSat = (ulong)100000;
            // Optionally, set a redirect URL for after the purchase is completed
            var optionalRedirectUrl = "https://example.com/purchase-complete";

            var request = new BuyBitcoinRequest.Moonpay(
                lockedAmountSat: optionalLockedAmountSat,
                redirectUrl: optionalRedirectUrl
            );

            var response = await sdk.BuyBitcoin(request: request);
            Console.WriteLine("Open this URL in a browser to complete the purchase:");
            Console.WriteLine($"{response.url}");
            // ANCHOR_END: buy-bitcoin
        }

        async Task BuyBitcoinViaCashapp(BreezSdk sdk)
        {
            // ANCHOR: buy-bitcoin-cashapp
            var request = new BuyBitcoinRequest.CashApp(
                amountSats: null
            );

            var response = await sdk.BuyBitcoin(request: request);
            Console.WriteLine("Open this URL in Cash App to complete the purchase:");
            Console.WriteLine($"{response.url}");
            // ANCHOR_END: buy-bitcoin-cashapp
        }
    }
}
