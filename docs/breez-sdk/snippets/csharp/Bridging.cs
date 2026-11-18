using System.Numerics;
using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class Bridging
    {
        async Task<BridgingClient> NewClient()
        {
            // ANCHOR: new-bridging-client
            var config = BreezSdkSparkMethods.DefaultBridgingConfig(apiKey: "<your api key>");
            var client = await BreezSdkSparkMethods.NewBridgingClient(config: config);
            // ANCHOR_END: new-bridging-client
            return client;
        }

        BridgingClient ClientFromSdk(BreezSdk sdk)
        {
            // ANCHOR: get-bridging-client
            var client = sdk.GetBridgingClient();
            // ANCHOR_END: get-bridging-client
            return client;
        }

        async Task CreateBridge(BridgingClient client)
        {
            // ANCHOR: create-bridge
            // Who to pay: a Spark address, a Lightning address or an EVM/Solana/Tron
            // address.
            var recipient = "<recipient>";

            // List the routes that pay them and pick what the payer pays in, e.g.
            // USDC on Base.
            var routes = (await client.GetRoutes(
                request: new GetBridgeRoutesRequest(recipient: recipient)
            )).routes;
            var route = routes.First(
                r => r.source.asset == "USDC" && r.source.chain == "base"
            );

            // Ask for $10. The amount is in the recipient's stablecoin when they are
            // paid in one, the payer's otherwise (6 decimals for USDC). The deposit is
            // refunded to the payer's address if the recipient can't be paid.
            var request = new CreateBridgeRequest(
                recipient: recipient,
                route: route,
                amount: new BigInteger(10_000_000),
                // Required to pay a Lightning address.
                refundAddress: "<payer address on Base>",
                feePolicy: null,
                maxSlippageBps: null
            );
            var response = await client.CreateBridge(request: request);

            // Show the payer what to pay, and store the bridge id and status token to
            // look up the status later.
            Console.WriteLine($"Pay with: {response.paymentRequest}");
            Console.WriteLine($"Bridge id: {response.bridgeId}");
            // ANCHOR_END: create-bridge
        }

        async Task GetBridgeStatus(
            BridgingClient client,
            string bridgeId,
            string? statusToken
        )
        {
            // ANCHOR: get-bridge-status
            var response = await client.GetBridgeStatus(
                request: new GetBridgeStatusRequest(bridgeId: bridgeId, statusToken: statusToken)
            );
            switch (response.status)
            {
                case BridgeStatus.Completed:
                    Console.WriteLine($"Delivered {response.amountOut}");
                    break;
                case BridgeStatus.Refunded:
                case BridgeStatus.Failed:
                case BridgeStatus.Expired:
                    Console.WriteLine($"Not delivered: {response.status}");
                    break;
                default:
                    Console.WriteLine($"Not final yet: {response.status}");
                    break;
            }
            // ANCHOR_END: get-bridge-status
        }
    }
}
