import BigNumber
import BreezSdkSpark
import Foundation

func getIssuerSdk(sdk: BreezSdk) -> BreezIssuerSdk {
    // ANCHOR: get-issuer-sdk
    let issuerSdk = sdk.getIssuerSdk()
    // ANCHOR_END: get-issuer-sdk
    return issuerSdk
}

func createToken(issuerSdk: BreezIssuerSdk) async throws -> TokenMetadata {
    // ANCHOR: create-token
    let request = CreateIssuerTokenRequest(
        name: "My Token",
        ticker: "MTK",
        decimals: UInt32(6),
        isFreezable: true,
        maxSupply: BInt(1_000_000)
    )
    let tokenMetadata = try await issuerSdk.createIssuerToken(request: request)
    print("Token identifier: {}", tokenMetadata.identifier)
    // ANCHOR_END: create-token
    return tokenMetadata
}

func mintToken(issuerSdk: BreezIssuerSdk) async throws -> Payment {
    // ANCHOR: mint-token
    let request = MintIssuerTokenRequest(
        amount: BInt(1_000)
    )
    let payment = try await issuerSdk.mintIssuerToken(request: request)
    // ANCHOR_END: mint-token
    return payment
}

func burnToken(issuerSdk: BreezIssuerSdk) async throws -> Payment {
    // ANCHOR: burn-token
    let request = BurnIssuerTokenRequest(
        amount: BInt(1_000)
    )
    let payment = try await issuerSdk.burnIssuerToken(request: request)
    // ANCHOR_END: burn-token
    return payment
}

func getTokenMetadata(issuerSdk: BreezIssuerSdk) async throws -> TokenMetadata {
    // ANCHOR: get-token-metadata
    let tokenBalance = try await issuerSdk.getIssuerTokenBalance()
    print("Token balance: {}", tokenBalance.balance)

    let tokenMetadata = try await issuerSdk.getIssuerTokenMetadata()
    print("Token ticker: {}", tokenMetadata.ticker)
    // ANCHOR_END: get-token-metadata
    return tokenMetadata
}

func freezeToken(issuerSdk: BreezIssuerSdk) async throws {
    // ANCHOR: freeze-token
    let sparkAddress = "<spark address>"
    // Freeze the tokens held at the specified Spark address
    let freezeRequest = FreezeIssuerTokenRequest(
        address: sparkAddress
    )
    let freezeResponse = try await issuerSdk.freezeIssuerToken(request: freezeRequest)

    // Unfreeze the tokens held at the specified Spark address
    let unfreezeRequest = UnfreezeIssuerTokenRequest(
        address: sparkAddress
    )
    let unfreezeResponse = try await issuerSdk.unfreezeIssuerToken(request: unfreezeRequest)
    // ANCHOR_END: freeze-token
}
