using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    public class ExternalSignerSnippets
    {
        // ANCHOR: default-external-signer
        public static ExternalSigners CreateSigners()
        {
            var mnemonic = "<mnemonic words>";
            var network = Network.Mainnet;
            uint accountNumber = 0;

            var signers = BreezSdkSparkMethods.DefaultExternalSigners(
                mnemonic: mnemonic,
                passphrase: null,
                network: network,
                accountNumber: accountNumber
            );

            return signers;
        }
        // ANCHOR_END: default-external-signer

        // ANCHOR: connect-with-signer
        public static async Task<BreezSdk> ConnectWithSigner(ExternalSigners signers)
        {
            // Create the config
            var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
            {
                apiKey = "<breez api key>"
            };

            // Connect using the external signers
            var sdk = await BreezSdkSparkMethods.ConnectWithSigner(new ConnectWithSignerRequest(
                config: config,
                breezSigner: signers.breezSigner,
                sparkSigner: signers.sparkSigner,
                storageDir: "./.data"
            ));

            return sdk;
        }
        // ANCHOR_END: connect-with-signer

        // ANCHOR: sdk-builder-with-signer
        public static async Task<BreezSdk> BuildWithSigner(ExternalSigners signers)
        {
            var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
            {
                apiKey = "<breez api key>"
            };

            var builder = SdkBuilder.NewWithSigner(
                config: config,
                breezSigner: signers.breezSigner,
                sparkSigner: signers.sparkSigner
            );
            // await builder.WithStorageBackend(storage: <your storage backend>);
            // await builder.WithSharedContext(<your shared context>);
            var sdk = await builder.Build();

            return sdk;
        }
        // ANCHOR_END: sdk-builder-with-signer

        // ANCHOR: sdk-builder-with-signing-only-signer
        public static async Task<BreezSdk> BuildWithSigningOnlySigner(Config config, SigningOnlyExternalSigners signers)
        {
            var builder = SdkBuilder.NewWithSigningOnlySigner(
                config: config,
                breezSigner: signers.breezSigner,
                sparkSigner: signers.sparkSigner
            );
            var sdk = await builder.Build();

            return sdk;
        }
        // ANCHOR_END: sdk-builder-with-signing-only-signer
    }
}
