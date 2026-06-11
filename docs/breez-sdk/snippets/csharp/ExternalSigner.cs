using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    public class ExternalSignerSnippets
    {
        // ANCHOR: default-external-signer
        public static DefaultExternalSigners CreateSigners()
        {
            var mnemonic = "<mnemonic words>";
            var network = Network.Mainnet;
            uint accountNumber = 0;

            var keySetConfig = new KeySetConfig(
                accountNumber: accountNumber
            );

            var signers = BreezSdkSparkMethods.DefaultExternalSigners(
                mnemonic: mnemonic,
                passphrase: null,
                network: network,
                keySetConfig: keySetConfig
            );

            return signers;
        }
        // ANCHOR_END: default-external-signer

        // ANCHOR: connect-with-signer
        public static async Task<BreezSdk> ConnectWithSigner(DefaultExternalSigners signers)
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
    }
}
