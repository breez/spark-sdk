import BreezSdkSpark
import Foundation

func configureLightningAddress() -> Config {
    // ANCHOR: config-lightning-address
    var config = defaultConfig(network: Network.mainnet)
    config.apiKey = "your-api-key"
    config.lnurlDomain = "yourdomain.com"
    // ANCHOR_END: config-lightning-address
    return config
}

func checkLightningAddressAvailability(sdk: BreezSdk) async throws {
    let username = "myusername"
    
    // ANCHOR: check-lightning-address
    let request = CheckLightningAddressRequest(
        username: username
    )
    
    let available = try await sdk.checkLightningAddressAvailable(req: request)
    // ANCHOR_END: check-lightning-address
}

func registerLightningAddress(sdk: BreezSdk) async throws {
    let username = "myusername"
    let description = "My Lightning Address"
    
    // ANCHOR: register-lightning-address
    let request = RegisterLightningAddressRequest(
        username: username,
        description: description
    )
    
    let addressInfo = try await sdk.registerLightningAddress(request: request)
    let lightningAddress = addressInfo.lightningAddress
    let lnurlUrl = addressInfo.lnurl.url
    let lnurlBech32 = addressInfo.lnurl.bech32
    // ANCHOR_END: register-lightning-address
}

func getLightningAddress(sdk: BreezSdk) async throws {
    // ANCHOR: get-lightning-address
    if let addressInfo = try await sdk.getLightningAddress() {
        let lightningAddress = addressInfo.lightningAddress
        let username = addressInfo.username
        let description = addressInfo.description
        let lnurlUrl = addressInfo.lnurl.url
        let lnurlBech32 = addressInfo.lnurl.bech32
    }
    // ANCHOR_END: get-lightning-address
}

// Run on the *current owner's* wallet. Produces the authorization that the
// new owner needs to take over the username in a single atomic call.
func signLightningAddressTransfer(
    currentOwnerSdk: BreezSdk,
    currentOwnerPubkey: String,
    newOwnerPubkey: String
) async throws -> LightningAddressTransfer {
    let username = "myusername"

    // ANCHOR: sign-lightning-address-transfer
    // `username` must be lowercased and trimmed.
    // pubkeys are hex-encoded secp256k1 compressed (via getInfo().identityPubkey).
    let message = "transfer:\(currentOwnerPubkey)-\(username)-\(newOwnerPubkey)"
    let signed = try await currentOwnerSdk.signMessage(
        request: SignMessageRequest(message: message, compact: false)
    )

    let transfer = LightningAddressTransfer(
        pubkey: signed.pubkey,
        signature: signed.signature
    )
    // ANCHOR_END: sign-lightning-address-transfer
    return transfer
}

// Run on the *new owner's* wallet with the authorization received
// out-of-band from the current owner.
func registerLightningAddressViaTransfer(
    newOwnerSdk: BreezSdk,
    transfer: LightningAddressTransfer
) async throws {
    let username = "myusername"
    let description = "My Lightning Address"

    // ANCHOR: register-lightning-address-transfer
    let request = RegisterLightningAddressRequest(
        username: username,
        description: description,
        transfer: transfer
    )

    let addressInfo = try await newOwnerSdk.registerLightningAddress(request: request)
    // ANCHOR_END: register-lightning-address-transfer
}

func deleteLightningAddress(sdk: BreezSdk) async throws {
    // ANCHOR: delete-lightning-address
    try await sdk.deleteLightningAddress()
    // ANCHOR_END: delete-lightning-address
}

func accessSenderComment(sdk: BreezSdk) async throws {
    let paymentId = "<payment id>"
    let response = try await sdk.getPayment(request: GetPaymentRequest(paymentId: paymentId))
    let payment = response.payment
    
    // ANCHOR: access-sender-comment
    // Check if this is a lightning payment with LNURL receive metadata
    if case .lightning(let details) = payment.details {
        // Access the sender comment if present
        if let metadata = details.lnurlReceiveMetadata,
           let comment = metadata.senderComment {
            print("Sender comment: \(comment)")
        }
    }
    // ANCHOR_END: access-sender-comment
}

func accessNostrZap(sdk: BreezSdk) async throws {
    let paymentId = "<payment id>"
    let response = try await sdk.getPayment(request: GetPaymentRequest(paymentId: paymentId))
    let payment = response.payment
    
    // ANCHOR: access-nostr-zap
    // Check if this is a lightning payment with LNURL receive metadata
    if case .lightning(let details) = payment.details {
        if let metadata = details.lnurlReceiveMetadata {
            // Access the Nostr zap request if present
            if let zapRequest = metadata.nostrZapRequest {
                // The zapRequest is a JSON string containing the Nostr event (kind 9734)
                print("Nostr zap request: \(zapRequest)")
            }
            
            // Access the Nostr zap receipt if present
            if let zapReceipt = metadata.nostrZapReceipt {
                // The zapReceipt is a JSON string containing the Nostr event (kind 9735)
                print("Nostr zap receipt: \(zapReceipt)")
            }
        }
    }
    // ANCHOR_END: access-nostr-zap
}
