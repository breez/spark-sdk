using System.Numerics;
using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class CrossChain
    {
        async Task GetCrossChainRoutes(BreezSdk sdk)
        {
            // ANCHOR: cross-chain-get-routes
            var inputStr = "<recipient address>";
            var parsed = await sdk.Parse(input: inputStr);
            if (parsed is not InputType.CrossChainAddress crossChain)
            {
                throw new InvalidOperationException("Not a cross-chain address");
            }
            var addressDetails = crossChain.v1;

            var filter = new CrossChainRouteFilter.Send(addressDetails: addressDetails);
            var routes = await sdk.GetCrossChainRoutes(filter: filter);

            foreach (var route in routes)
            {
                Console.WriteLine($"Route via {route.provider}: {route.chain}/{route.asset}");
            }
            // ANCHOR_END: cross-chain-get-routes
        }

        async Task PrepareSendPaymentCrossChain(
            BreezSdk sdk,
            CrossChainAddressDetails addressDetails,
            CrossChainRoutePair route)
        {
            // ANCHOR: cross-chain-prepare
            // Optionally set the maximum slippage in basis points (10 to 500)
            uint? optionalMaxSlippageBps = 100;

            var request = new PrepareSendPaymentRequest(
                paymentRequest: new PaymentRequest.CrossChain(
                    address: addressDetails.address,
                    route: route,
                    maxSlippageBps: optionalMaxSlippageBps,
                    targetOverpayBps: null
                ),
                amount: 50_000UL,
                tokenIdentifier: null,
                conversionOptions: null,
                feePolicy: null
            );
            var prepareResponse = await sdk.PrepareSendPayment(request: request);

            if (prepareResponse.paymentMethod is SendPaymentMethod.CrossChainAddress method)
            {
                Console.WriteLine($"Amount in: {method.amountIn}");
                Console.WriteLine($"Estimated out: {method.estimatedOut}");
                Console.WriteLine($"Provider fee: {method.feeAmount}");
                Console.WriteLine($"Quote expires at: {method.expiresAt}");
            }
            // ANCHOR_END: cross-chain-prepare
        }

        async Task SendPaymentCrossChain(BreezSdk sdk, PrepareSendPaymentResponse prepareResponse)
        {
            // ANCHOR: cross-chain-send
            // Only valid for sends with no token leg (see Retry safety).
            var optionalIdempotencyKey = "<idempotency key uuid>";
            var request = new SendPaymentRequest(
                prepareResponse: prepareResponse,
                options: null,
                idempotencyKey: optionalIdempotencyKey
            );
            var sendResponse = await sdk.SendPayment(request: request);
            Console.WriteLine($"Payment: {sendResponse.payment}");
            // ANCHOR_END: cross-chain-send
        }

        async Task GetCrossChainReceiveRoutes(BreezSdk sdk)
        {
            // ANCHOR: cross-chain-get-receive-routes
            var filter = new CrossChainRouteFilter.Receive(contractAddress: null);
            var routes = await sdk.GetCrossChainRoutes(filter: filter);

            foreach (var route in routes)
            {
                Console.WriteLine(
                    $"Route via {route.provider}: {route.chain}/{route.asset} -> Spark"
                );
            }
            // ANCHOR_END: cross-chain-get-receive-routes
        }

        async Task ReceivePaymentCrossChain(BreezSdk sdk, CrossChainRoutePair route)
        {
            // ANCHOR: cross-chain-receive
            // With the default FeesExcluded mode, amount is the receiver's net
            // target on Spark in destination-asset base units (sats for BTC,
            // token base units for USDB). The SDK pads the sender's deposit to
            // cover fees + overpay. With FeesIncluded, amount is the sender's
            // deposit in source-asset units.
            var amount = new BigInteger(1_000);
            // Optionally set the destination Spark-side asset. null = auto:
            // active stable-balance token if the route supports it, otherwise BTC.
            SparkAsset? optionalDestination = null;
            // Optionally set the maximum slippage in basis points (10 to 500)
            uint? optionalMaxSlippageBps = 100;
            // Optionally override the overpay buffer (0 to 500 bps). Defaults to 15.
            uint? optionalTargetOverpayBps = null;
            // Optionally override the fee mode. Defaults to FeesExcluded.
            CrossChainFeeMode? optionalFeeMode = null;

            var request = new ReceivePaymentRequest(
                paymentMethod: new ReceivePaymentMethod.CrossChain(
                    route: route,
                    amount: amount,
                    destination: optionalDestination,
                    feeMode: optionalFeeMode,
                    maxSlippageBps: optionalMaxSlippageBps,
                    targetOverpayBps: optionalTargetOverpayBps
                )
            );
            var response = await sdk.ReceivePayment(request: request);

            Console.WriteLine($"Payment request: {response.paymentRequest}");
            if (response.crossChainInfo is { } info)
            {
                Console.WriteLine($"Deposit address: {info.depositAddress}");
                Console.WriteLine($"Deposit amount: {info.depositAmount}");
                var denom = info.tokenIdentifier is null ? "BTC" : "USDB";
                Console.WriteLine(
                    $"Expected received: {info.expectedReceivedAmount} {denom}"
                );
                Console.WriteLine($"Expires at: {info.expiresAt}");
            }
            // ANCHOR_END: cross-chain-receive
        }
    }
}
