using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    public class TurnkeySnippets
    {
        public static async Task<BreezSdk> ConnectWithTurnkey()
        {
            // ANCHOR: turnkey-connect
            var turnkeyConfig = new TurnkeyConfig(
                baseUrl: null,
                organizationId: "<turnkey sub-organization id>",
                apiPublicKey: "<api public key hex>",
                apiPrivateKey: "<api private key hex>",
                walletId: "<turnkey wallet id>",
                network: Network.Mainnet,
                accountNumber: null,
                // Set after the first connect to make later signer setup network-free
                identityPublicKey: null,
                retry: null,
                maxRps: null
            );

            var signers = await BreezSdkSparkMethods.CreateTurnkeySigner(turnkeyConfig);

            var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
            {
                apiKey = "<breez api key>"
            };

            var sdk = await BreezSdkSparkMethods.ConnectWithSigner(new ConnectWithSignerRequest(
                config: config,
                breezSigner: signers.breezSigner,
                sparkSigner: signers.sparkSigner,
                storageDir: "./.data"
            ));
            // ANCHOR_END: turnkey-connect
            return sdk;
        }
    }
}
