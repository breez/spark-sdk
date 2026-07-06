import Foundation
import BreezSdkSpark

class ExternalSignerSnippets {
    // ANCHOR: default-external-signer
    func createSigners() throws -> ExternalSigners {
        let mnemonic = "<mnemonic words>"
        let network = Network.mainnet
        
        let signers = try defaultExternalSigners(
            mnemonic: mnemonic,
            passphrase: nil,
            network: network,
            accountNumber: 0
        )
        
        return signers
    }
    // ANCHOR_END: default-external-signer
    
    // ANCHOR: connect-with-signer
    func connectExample(signers: ExternalSigners) async throws -> BreezSdk {
        // Create the config
        var config = defaultConfig(network: .mainnet)
        config.apiKey = "<breez api key>"

        // Connect using the external signers
        let sdk = try await BreezSdkSpark.connectWithSigner(request: ConnectWithSignerRequest(
            config: config,
            breezSigner: signers.breezSigner,
            sparkSigner: signers.sparkSigner,
            storageDir: "./.data"
        ))
        
        return sdk
    }
    // ANCHOR_END: connect-with-signer
}
