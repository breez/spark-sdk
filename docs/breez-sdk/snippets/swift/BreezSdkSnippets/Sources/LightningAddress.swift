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
    let lnurl = addressInfo.lnurl
    // ANCHOR_END: register-lightning-address
}

func getLightningAddress(sdk: BreezSdk) async throws {
    // ANCHOR: get-lightning-address
    if let addressInfo = try await sdk.getLightningAddress() {
        let lightningAddress = addressInfo.lightningAddress
        let username = addressInfo.username
        let description = addressInfo.description
        let lnurl = addressInfo.lnurl
    }
    // ANCHOR_END: get-lightning-address
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
