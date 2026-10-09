using System.Numerics;
using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class CashApp
    {
        async Task BridgeFromCashApp(BreezSdk sdk)
        {
            // ANCHOR: bridge-from-cash-app
            // Parse the recipient's external-chain address (EVM/Solana/Tron).
            var parsed = await sdk.Parse(input: "<recipient address>");
            if (parsed is not InputType.CrossChainAddress crossChain)
            {
                throw new InvalidOperationException("Not a cross-chain address");
            }
            var addressDetails = crossChain.v1;

            // List the stablecoin destinations Cash App can fund over Lightning
            // and pick one, e.g. USDC on Base.
            var filter = new CrossChainRouteFilter.Send(
                addressDetails: addressDetails,
                deliveryMethod: DeliveryMethod.Lightning
            );
            var routes = await sdk.GetCrossChainRoutes(filter: filter);
            var route = routes.First(r => r.asset == "USDC" && r.chain == "base");

            // Send $10 of USDC, funded by Cash App over Lightning. The amount
            // is in the route asset's base units (USDC, 6 decimals), so
            // 10_000_000 = 10 USDC, about $10.
            var request = new BridgeFromCashAppRequest(
                address: addressDetails.address,
                route: route,
                amount: new BigInteger(10_000_000),
                feePolicy: null,
                maxSlippageBps: null
            );
            var response = await sdk.BridgeFromCashApp(request: request);

            // Open this Cash App URL to pay. The recipient then receives the
            // stablecoin.
            Console.WriteLine($"Open this URL in Cash App: {response.url}");
            Console.WriteLine($"Recipient receives ~{response.estimatedOut} {response.asset}");
            // ANCHOR_END: bridge-from-cash-app
        }

        async Task BridgeToCashApp(BreezSdk sdk)
        {
            // ANCHOR: bridge-to-cash-app
            // List the stablecoin sources that can pay a Cash App user over
            // Lightning and pick one, e.g. USDC on Base.
            var filter = new CrossChainRouteFilter.Receive(
                contractAddress: null,
                deliveryMethod: DeliveryMethod.Lightning
            );
            var routes = await sdk.GetCrossChainRoutes(filter: filter);
            var route = routes.First(r => r.asset == "USDC" && r.chain == "base");

            // Pay $10 of USDC to the Cash App user $alice. The amount is in
            // the route asset's base units (USDC, 6 decimals), so
            // 10_000_000 = 10 USDC, about $10. The deposit is refunded to the
            // payer's address if delivery fails.
            var request = new BridgeToCashAppRequest(
                recipient: "$alice",
                route: route,
                amount: new BigInteger(10_000_000),
                feePolicy: null,
                refundAddress: "<payer address>",
                maxSlippageBps: null
            );
            var response = await sdk.BridgeToCashApp(request: request);

            // Show the payer what to pay. The recipient then receives Bitcoin.
            var info = response.info;
            Console.WriteLine($"Pay with: {response.paymentRequest}");
            Console.WriteLine(
                $"Deposit {info.depositAmount} to {info.depositAddress}, "
                    + $"recipient receives ~{info.expectedReceivedAmount} sats"
            );
            // ANCHOR_END: bridge-to-cash-app
        }
    }
}
