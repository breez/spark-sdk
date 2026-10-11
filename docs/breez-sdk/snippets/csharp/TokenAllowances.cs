using System.Numerics;
using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class TokenAllowances
    {
        async Task CreateTokenAllowance(BreezSdk sdk)
        {
            // ANCHOR: create-token-allowance
            var response = await sdk.CreateTokenAllowance(
                request: new CreateTokenAllowanceRequest(
                    spenderAddress: "<spender spark address>",
                    tokenIdentifier: "<token identifier>",
                    maxPerPayment: new TokenAllowanceLimit.Amount(new BigInteger(5_000_000)),
                    maxTotal: new TokenAllowanceLimit.Amount(new BigInteger(100_000_000)),
                    expiryTime: 1_798_761_600UL,
                    allowedRecipients: new string[] { }
                )
            );
            Console.WriteLine($"Allowance id: {response.allowance.id}");
            // ANCHOR_END: create-token-allowance
        }

        async Task ListTokenAllowances(BreezSdk sdk)
        {
            // ANCHOR: list-token-allowances
            var response = await sdk.ListTokenAllowances(
                request: new ListTokenAllowancesRequest(
                    role: TokenAllowanceRole.Owner,
                    counterpartyAddress: null,
                    tokenIdentifier: null,
                    includeInactive: false,
                    offset: null,
                    limit: null
                )
            );
            foreach (var allowance in response.allowances)
            {
                Console.WriteLine($"{allowance.id}: spent {allowance.spentAmount}");
            }
            // ANCHOR_END: list-token-allowances
        }

        async Task RevokeTokenAllowance(BreezSdk sdk)
        {
            // ANCHOR: revoke-token-allowance
            await sdk.RevokeTokenAllowance(
                request: new RevokeTokenAllowanceRequest(allowanceId: "<allowance id>")
            );
            // ANCHOR_END: revoke-token-allowance
        }

        async Task PullPayment(BreezSdk sdk)
        {
            // ANCHOR: pull-payment
            var prepareResponse = await sdk.PreparePullPayment(
                request: new PreparePullPaymentRequest(
                    payerAddress: "<payer spark address>",
                    tokenIdentifier: "<token identifier>",
                    receivers: new PullReceiver[] {
                        new PullReceiver(
                            amount: new BigInteger(5_000_000),
                            receiverAddress: null
                        )
                    }
                )
            );
            Console.WriteLine($"Pulling {prepareResponse.amount}");

            var response = await sdk.PullPayment(
                request: new PullPaymentRequest(prepareResponse: prepareResponse)
            );
            Console.WriteLine($"Pull transaction: {response.txHash}");
            // ANCHOR_END: pull-payment
        }
    }
}
