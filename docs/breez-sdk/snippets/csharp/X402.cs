using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class X402Snippets
    {
        X402 GetX402(BreezSdk sdk)
        {
            // ANCHOR: get-x402
            var x402 = sdk.GetX402();
            // ANCHOR_END: get-x402
            return x402;
        }

        async Task PayHttpRequest(X402 x402, string paymentRequiredHeader)
        {
            // ANCHOR: pay-http-request
            // The request that returned 402, exactly as it will be sent again
            var request = new X402Request.Http(
                method: "GET",
                url: "https://api.example.com/article/A",
                headers: new X402Header[] {
                    new X402Header(name: "accept", value: "application/json")
                },
                body: null
            );
            var response = await x402.Pay(request: new PayX402Request(
                // The PAYMENT-REQUIRED header of the 402 response
                paymentRequired: paymentRequiredHeader,
                request: request,
                maxAmount: new X402PaymentAmount.Lightning(amountSats: 100),
                conversionOptions: null,
                completionTimeoutSecs: null
            ));

            if (response.paymentSignature != null)
            {
                // Send the request again with it as the PAYMENT-SIGNATURE header.
                // It proves the payment, so don't log it.
            }
            else
            {
                // Still in flight: paying the same challenge again picks it up
                Console.WriteLine($"Payment status: {response.status}");
            }
            // ANCHOR_END: pay-http-request
        }

        async Task PayMcpToolCall(X402 x402, string structuredContent)
        {
            // ANCHOR: pay-mcp-tool-call
            var request = new X402Request.Mcp(
                server: "https://api.example.com/mcp",
                // The tools/call params, exactly as they will be sent again
                @params: """{"name":"get_article","arguments":{"article":"A"}}"""
            );
            var response = await x402.Pay(request: new PayX402Request(
                // The structuredContent of the tool result
                paymentRequired: structuredContent,
                request: request,
                maxAmount: new X402PaymentAmount.Lightning(amountSats: 100),
                conversionOptions: null,
                completionTimeoutSecs: null
            ));

            if (response.paymentSignature != null)
            {
                // Call the tool again with this JSON in params._meta["x402/payment"]
            }
            // ANCHOR_END: pay-mcp-tool-call
        }

        async Task CreateChallenge(X402 x402)
        {
            // ANCHOR: create-challenge
            // The unpaid request, with its URL built from your public origin
            var request = new X402Request.Http(
                method: "GET",
                url: "https://api.example.com/article/A",
                headers: new X402Header[] {
                    new X402Header(name: "accept", value: "application/json")
                },
                body: null
            );
            var response = await x402.CreateChallenge(request: new CreateX402ChallengeRequest(
                request: request,
                // Headers that change what is served, so the payment covers them too
                boundNames: new string[] { "accept" },
                amounts: new X402PaymentAmount[]
                {
                    new X402PaymentAmount.Lightning(amountSats: 21)
                },
                maxTimeoutSecs: null,
                description: "An article",
                mimeType: "application/json"
            ));

            // Respond with status 402 and this PAYMENT-REQUIRED header
            Console.WriteLine($"PAYMENT-REQUIRED: {response.paymentRequired}");
            // ANCHOR_END: create-challenge
        }

        async Task SettlePayment(X402 x402, string paymentSignatureHeader)
        {
            // ANCHOR: settle-payment
            // The paid request, exactly as received
            var request = new X402Request.Http(
                method: "GET",
                url: "https://api.example.com/article/A",
                headers: new X402Header[] {
                    new X402Header(name: "accept", value: "application/json")
                },
                body: null
            );
            var response = await x402.Settle(request: new SettleX402Request(
                // The PAYMENT-SIGNATURE header of the request
                paymentSignature: paymentSignatureHeader,
                request: request,
                boundNames: new string[] { "accept" },
                // The same price and lifetime the challenge was created with
                amounts: new X402PaymentAmount[]
                {
                    new X402PaymentAmount.Lightning(amountSats: 21)
                },
                maxTimeoutSecs: null
            ));

            // Send the PAYMENT-RESPONSE header whatever the status
            Console.WriteLine($"PAYMENT-RESPONSE: {response.paymentResponse}");
            switch (response.status)
            {
                case X402SettleStatus.Settled:
                    // Serve the resource
                    break;
                case X402SettleStatus.Pending:
                    // Respond with status 402 and no new challenge
                    break;
                case X402SettleStatus.Malformed:
                    // Respond with status 400
                    break;
                case X402SettleStatus.Refused:
                    // Respond with status 402 and a new challenge
                    break;
                case X402SettleStatus.Error:
                    // Respond with status 500
                    break;
            }
            // ANCHOR_END: settle-payment
        }
    }
}
