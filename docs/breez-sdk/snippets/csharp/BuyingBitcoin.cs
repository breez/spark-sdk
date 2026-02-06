using BreezSdkSpark;

namespace BreezSdkSnippets
{
    class BuyingBitcoin
    {
        async Task BuyBitcoinBasic(BreezSdk sdk)
        {
            // ANCHOR: buy-bitcoin-basic
            // Buy Bitcoin using the SDK's auto-generated deposit address
            var request = new BuyBitcoinRequest();

            var response = await sdk.BuyBitcoin(request: request);
            Console.WriteLine("Open this URL in a browser to complete the purchase:");
            Console.WriteLine($"{response.url}");
            // ANCHOR_END: buy-bitcoin-basic
        }

        async Task BuyBitcoinWithAmount(BreezSdk sdk)
        {
            // ANCHOR: buy-bitcoin-with-amount
            // Lock the purchase to a specific amount (e.g., 0.001 BTC = 100,000 sats)
            var request = new BuyBitcoinRequest(
                lockedAmountSat: 100000
            );

            var response = await sdk.BuyBitcoin(request: request);
            Console.WriteLine("Open this URL in a browser to complete the purchase:");
            Console.WriteLine($"{response.url}");
            // ANCHOR_END: buy-bitcoin-with-amount
        }

        async Task BuyBitcoinWithRedirect(BreezSdk sdk)
        {
            // ANCHOR: buy-bitcoin-with-redirect
            // Provide a custom redirect URL for after the purchase
            var request = new BuyBitcoinRequest(
                lockedAmountSat: 100000,
                redirectUrl: "https://example.com/purchase-complete"
            );

            var response = await sdk.BuyBitcoin(request: request);
            Console.WriteLine("Open this URL in a browser to complete the purchase:");
            Console.WriteLine($"{response.url}");
            // ANCHOR_END: buy-bitcoin-with-redirect
        }

    }
}
