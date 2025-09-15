import BreezSdkSpark
import Foundation

func checkLightningAddressAvailability(sdk: BreezSdk) async throws {
    let username = "myusername"
    
    // ANCHOR: check-lightning-address
    let request = CheckLightningAddressRequest(
        username: username
    )
    
    let available = try await sdk.checkLightningAddressAvailable(request: request)
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
