import Foundation
import BreezSdkSpark

class ExternalSignerSnippets {
    // ANCHOR: default-external-signer
    func createSigner() throws -> ExternalSigner {
        let mnemonic = "<mnemonic words>"
        let network = Network.mainnet
        let keySetType = KeySetType.default
        let useAddressIndex = false
        let accountNumber: UInt32 = 0
        
        let signer = try defaultExternalSigner(
            mnemonic: mnemonic,
            passphrase: nil,
            network: network,
            keySetType: keySetType,
            useAddressIndex: useAddressIndex,
            accountNumber: accountNumber
        )
        
        return signer
    }
    // ANCHOR_END: default-external-signer
    
    // ANCHOR: connect-with-signer
    func connectWithSigner() async throws -> BreezSdk {
        // Create the signer
        let signer = try! defaultExternalSigner(
            mnemonic: "<mnemonic words>",
            passphrase: nil,
            network: Network.mainnet,
            keySetConfig: KeySetConfig(
                keySetType: KeySetType.default,
                useAddressIndex: false,
                accountNumber: 0
            )
        )

        
        // Create the config
        var config = defaultConfig(network: .mainnet)
        config.apiKey = "<breez api key>"
        
        // Connect using the external signer
        let sdk = try await connectWithSigner(request: ConnectWithSignerRequest(
            config: config,
            signer: signer,
            storageDir: "./.data"
        ))
        
        return sdk
    }
    // ANCHOR_END: connect-with-signer
}
