using System.Numerics;
using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class PreparePaymentLink
    {
        async Task PreparePaymentLinkViaCashapp(BreezSdk sdk)
        {
            // ANCHOR: prepare-payment-link-cashapp
            // Parse the recipient's external-chain address (EVM/Solana/Tron).
            var parsed = await sdk.Parse(input: "<recipient address>");
            if (parsed is not InputType.CrossChainAddress crossChain)
            {
                throw new InvalidOperationException("Not a cross-chain address");
            }
            var addressDetails = crossChain.v1;

            // List the stablecoin destinations you can send to and pick one,
            // e.g. USDC on Base. Restrict to Lightning-fundable routes,
            // since Cash App funds over Lightning.
            var filter = new CrossChainRouteFilter.PaymentLink(
                addressDetails: addressDetails
            );
            var routes = await sdk.GetCrossChainRoutes(filter: filter);
            var route = routes.First(r => r.asset == "USDC" && r.chain == "base");

            // Send $10 of USDC (amount in USD 6-decimal base units,
            // 10_000_000 = $10.00), funded by Cash App over Lightning.
            var request = new PreparePaymentLinkRequest(
                address: addressDetails.address,
                route: route,
                amount: new BigInteger(10_000_000),
                feePolicy: null,
                maxSlippageBps: null
            );
            var response = await sdk.PreparePaymentLink(request: request);
            Console.WriteLine($"Open this URL in Cash App: {response.url}");
            Console.WriteLine($"Recipient receives ~{response.estimatedOut} {response.asset}");
            // ANCHOR_END: prepare-payment-link-cashapp
        }
    }
}
