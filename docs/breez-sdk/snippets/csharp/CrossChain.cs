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
    }
}
