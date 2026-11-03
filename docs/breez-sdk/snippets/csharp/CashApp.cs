using System.Numerics;
using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class CashApp
    {
        async Task BridgeFromCashApp(BridgingClient client)
        {
            // ANCHOR: bridge-from-cash-app
            // List the routes to the recipient's external-chain address
            // (EVM/Solana/Tron) and pick a stablecoin Cash App can fund over
            // Lightning, e.g. USDC on Base.
            var recipient = "<recipient address>";
            var routes = (await client.GetRoutes(
                request: new GetBridgeRoutesRequest(recipient: recipient)
            )).routes;
            var route = routes.First(
                r => r.source.chain == "lightning"
                    && r.destination.asset == "USDC"
                    && r.destination.chain == "base"
            );

            // Send $10 of USDC, funded by Cash App over Lightning. The amount is in
            // USDC base units (6 decimals), so 10_000_000 = 10 USDC, about $10.
            var request = new CreateBridgeRequest(
                recipient: recipient,
                route: route,
                amount: new BigInteger(10_000_000),
                refundAddress: null,
                feePolicy: null,
                maxSlippageBps: null
            );
            var response = await client.BridgeFromCashApp(request: request);

            // Open this Cash App link to pay. The recipient then receives the
            // stablecoin.
            var info = response.info;
            Console.WriteLine($"Open this URL in Cash App: {response.paymentRequest}");
            Console.WriteLine(
                $"Recipient receives ~{info.expectedReceivedAmount} {info.destinationAsset}"
            );
            // ANCHOR_END: bridge-from-cash-app
        }

        async Task BridgeToCashApp(BridgingClient client)
        {
            // ANCHOR: bridge-to-cash-app
            // List the routes to the Cash App user and pick what the payer pays in,
            // e.g. USDC on Base.
            var recipient = "$alice";
            var routes = (await client.GetRoutes(
                request: new GetBridgeRoutesRequest(recipient: recipient)
            )).routes;
            var route = routes.First(
                r => r.source.asset == "USDC" && r.source.chain == "base"
            );

            // Pay $10 of USDC to the Cash App user $alice. The amount is in USDC base
            // units (6 decimals), so 10_000_000 = 10 USDC, about $10. The deposit is
            // refunded to the payer's address if delivery fails.
            var request = new CreateBridgeRequest(
                recipient: recipient,
                route: route,
                amount: new BigInteger(10_000_000),
                refundAddress: "<payer address>",
                feePolicy: null,
                maxSlippageBps: null
            );
            var response = await client.BridgeToCashApp(request: request);

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
