using BreezSdkSpark;

namespace BreezSdkSnippets;

public class ExternalSignerSnippets
{
    // ANCHOR: default-external-signer
    public static ExternalSigner CreateSigner()
    {
        var mnemonic = "<mnemonic words>";
        var network = Network.Mainnet;
        var keySetType = KeySetType.Default;
        var useAddressIndex = false;
        uint accountNumber = 0;
        
        var signer = BreezSdkMethods.DefaultExternalSigner(
            mnemonic: mnemonic,
            passphrase: null,
            network: network,
            keySetType: keySetType,
            useAddressIndex: useAddressIndex,
            accountNumber: accountNumber
        );
        
        return signer;
    }
    // ANCHOR_END: default-external-signer
    
    // ANCHOR: connect-with-signer
    public static async Task<BreezSdk> ConnectWithSigner()
    {
        // Create the signer
        var signer = BreezSdkMethods.DefaultExternalSigner(
            mnemonic: "<mnemonic words>",
            passphrase: null,
            network: Network.Mainnet,
            keySetConfig: new KeySetConfig
            {
                keySetType = KeySetType.Default,
                useAddressIndex = false,
                accountNumber = 0
            }
        );
        
        // Create the config
        var config = BreezSdkMethods.DefaultConfig(Network.Mainnet);
        config.apiKey = "<breez api key>";
        
        // Connect using the external signer
        var sdk = await BreezSdkMethods.ConnectWithSigner(new ConnectWithSignerRequest
        {
            config = config,
            signer = signer,
            storageDir = "./.data"
        });
        
        return sdk;
    }
    // ANCHOR_END: connect-with-signer
}
