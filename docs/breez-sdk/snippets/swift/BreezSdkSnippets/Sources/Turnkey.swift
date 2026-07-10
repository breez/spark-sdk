import Foundation
import BreezSdkSpark

class TurnkeySnippets {
    func connectWithTurnkey() async throws -> BreezSdk {
        // ANCHOR: turnkey-connect
        let turnkeyConfig = TurnkeyConfig(
            baseUrl: nil,
            organizationId: "<turnkey sub-organization id>",
            apiPublicKey: "<api public key hex>",
            apiPrivateKey: "<api private key hex>",
            walletId: "<turnkey wallet id>",
            network: Network.mainnet,
            accountNumber: nil,
            // Set after the first connect to make later signer setup network-free
            identityPublicKey: nil,
            retry: nil,
            maxRps: nil
        )

        let signers = try await createTurnkeySigner(config: turnkeyConfig)

        var config = defaultConfig(network: Network.mainnet)
        config.apiKey = "<breez api key>"

        let sdk = try await BreezSdkSpark.connectWithSigner(request: ConnectWithSignerRequest(
            config: config,
            breezSigner: signers.breezSigner,
            sparkSigner: signers.sparkSigner,
            storageDir: "./.data"
        ))
        // ANCHOR_END: turnkey-connect
        return sdk
    }
}
