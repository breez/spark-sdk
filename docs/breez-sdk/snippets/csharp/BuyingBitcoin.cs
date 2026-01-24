using BreezSdkSpark;

namespace BreezSdkSnippets
{
    class BuyingBitcoin
    {
        async Task BuyBitcoinBasic(BreezSdk sdk)
        {
            // ANCHOR: buy-bitcoin-basic
            var request = new BuyBitcoinRequest(
                address: "bc1qexample...", // Your Bitcoin address
                lockedAmountSat: null,
                maxAmountSat: null,
                redirectUrl: null
            );

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
                address: "bc1qexample...",
                lockedAmountSat: 100000, // Pre-fill with 100,000 sats
                maxAmountSat: null,
                redirectUrl: null
            );

            var response = await sdk.BuyBitcoin(request: request);
            Console.WriteLine("Open this URL in a browser to complete the purchase:");
            Console.WriteLine($"{response.url}");
            // ANCHOR_END: buy-bitcoin-with-amount
        }

        async Task BuyBitcoinWithLimits(BreezSdk sdk)
        {
            // ANCHOR: buy-bitcoin-with-limits
            // Set both a locked amount and maximum amount
            var request = new BuyBitcoinRequest(
                address: "bc1qexample...",
                lockedAmountSat: 50000,   // Pre-fill with 50,000 sats
                maxAmountSat: 500000,     // Limit to 500,000 sats max
                redirectUrl: null
            );

            var response = await sdk.BuyBitcoin(request: request);
            Console.WriteLine("Open this URL in a browser to complete the purchase:");
            Console.WriteLine($"{response.url}");
            // ANCHOR_END: buy-bitcoin-with-limits
        }

        async Task BuyBitcoinWithRedirect(BreezSdk sdk)
        {
            // ANCHOR: buy-bitcoin-with-redirect
            // Provide a custom redirect URL for after the purchase
            var request = new BuyBitcoinRequest(
                address: "bc1qexample...",
                lockedAmountSat: 100000,
                maxAmountSat: null,
                redirectUrl: "https://example.com/purchase-complete"
            );

            var response = await sdk.BuyBitcoin(request: request);
            Console.WriteLine("Open this URL in a browser to complete the purchase:");
            Console.WriteLine($"{response.url}");
            // ANCHOR_END: buy-bitcoin-with-redirect
        }
    }
}
