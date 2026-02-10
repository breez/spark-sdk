using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class BuyingBitcoin
    {
        async Task BuyBitcoin(BreezSdk sdk)
        {
            // ANCHOR: buy-bitcoin
            // Buy Bitcoin with funds deposited directly into the user's wallet.
            // Optionally lock the purchase to a specific amount and provide a redirect URL.
            var request = new BuyBitcoinRequest(
                lockedAmountSat: 100000,
                redirectUrl: "https://example.com/purchase-complete"
            );

            var response = await sdk.BuyBitcoin(request: request);
            Console.WriteLine("Open this URL in a browser to complete the purchase:");
            Console.WriteLine($"{response.url}");
            // ANCHOR_END: buy-bitcoin
        }
    }
}
