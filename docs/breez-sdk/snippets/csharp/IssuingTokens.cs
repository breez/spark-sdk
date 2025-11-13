using System.Numerics;
using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class IssuingTokens
    {
        void GetTokenIssuer(BreezSdk sdk)
        {
            // ANCHOR: get-token-issuer
            var tokenIssuer = sdk.GetTokenIssuer();
            // ANCHOR_END: get-token-issuer
        }

        async Task CreateToken(TokenIssuer tokenIssuer)
        {
            // ANCHOR: create-token
            var maxSupply = new BigInteger(1000000);
            var request = new CreateIssuerTokenRequest(
                name: "My Token",
                ticker: "MTK",
                decimals: 6,
                isFreezable: false,
                maxSupply: maxSupply
            );

            var tokenMetadata = await tokenIssuer.CreateIssuerToken(request);
            Console.WriteLine($"Token identifier: {tokenMetadata.identifier}");
            // ANCHOR_END: create-token
        }

        async Task MintToken(TokenIssuer tokenIssuer)
        {
            // ANCHOR: mint-token
            var amount = new BigInteger(1000);
            var request = new MintIssuerTokenRequest(
                amount: amount
            );
            var payment = await tokenIssuer.MintIssuerToken(request);
            // ANCHOR_END: mint-token
        }

        async Task BurnToken(TokenIssuer tokenIssuer)
        {
            // ANCHOR: burn-token
            var amount = new BigInteger(1000);
            var request = new BurnIssuerTokenRequest(
                amount: amount
            );
            var payment = await tokenIssuer.BurnIssuerToken(request);
            // ANCHOR_END: burn-token
        }

        async Task GetTokenMetadata(TokenIssuer tokenIssuer)
        {
            // ANCHOR: get-token-metadata
            var tokenBalance = await tokenIssuer.GetIssuerTokenBalance();
            Console.WriteLine($"Token balance: {tokenBalance.balance}");

            var tokenMetadata = await tokenIssuer.GetIssuerTokenMetadata();
            Console.WriteLine($"Token ticker: {tokenMetadata.ticker}");
            // ANCHOR_END: get-token-metadata
        }

        async Task FreezeToken(TokenIssuer tokenIssuer)
        {
            // ANCHOR: freeze-token
            var sparkAddress = "<spark address>";
            var freezeRequest = new FreezeIssuerTokenRequest(
                address: sparkAddress
            );
            var freezeReponse = await tokenIssuer.FreezeIssuerToken(freezeRequest);

            var unfreezeRequest = new UnfreezeIssuerTokenRequest(
                address: sparkAddress
            );
            var unfreezeResponse = await tokenIssuer.UnfreezeIssuerToken(unfreezeRequest);
            // ANCHOR_END: freeze-token
        }
    }
}
