import BigNumber
import BreezSdkSpark
import Foundation

func getTokenIssuer(sdk: BreezSdk) -> TokenIssuer {
    // ANCHOR: get-token-issuer
    let tokenIssuer = sdk.getTokenIssuer()
    // ANCHOR_END: get-token-issuer
    return tokenIssuer
}

func createToken(tokenIssuer: TokenIssuer) async throws -> TokenMetadata {
    // ANCHOR: create-token
    let request = CreateIssuerTokenRequest(
        name: "My Token",
        ticker: "MTK",
        decimals: UInt32(6),
        isFreezable: false,
        maxSupply: BInt(1_000_000)
    )
    let tokenMetadata = try await tokenIssuer.createIssuerToken(request: request)
    print("Token identifier: {}", tokenMetadata.identifier)
    // ANCHOR_END: create-token
    return tokenMetadata
}

func createTokenWithCustomAccountNumber() async throws -> BreezSdk {
    // ANCHOR: custom-account-number
    let accountNumber = UInt32(21)

    let mnemonic = "<mnemonic words>"
    let seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: nil)
    let config = defaultConfig(network: Network.mainnet)
    let builder = SdkBuilder(config: config, seed: seed)
    await builder.withDefaultStorage(storageDir: "./.data")

    // Set the account number for the SDK
    await builder.withKeySet(
        keySetType: KeySetType.default, useAddressIndex: false, accountNumber: accountNumber)

    let sdk = try await builder.build()
    // ANCHOR_END: custom-account-number
    return sdk
}

func mintToken(tokenIssuer: TokenIssuer) async throws -> Payment {
    // ANCHOR: mint-token
    let request = MintIssuerTokenRequest(
        amount: BInt(1_000)
    )
    let payment = try await tokenIssuer.mintIssuerToken(request: request)
    // ANCHOR_END: mint-token
    return payment
}

func burnToken(tokenIssuer: TokenIssuer) async throws -> Payment {
    // ANCHOR: burn-token
    let request = BurnIssuerTokenRequest(
        amount: BInt(1_000)
    )
    let payment = try await tokenIssuer.burnIssuerToken(request: request)
    // ANCHOR_END: burn-token
    return payment
}

func getTokenMetadata(tokenIssuer: TokenIssuer) async throws -> TokenMetadata {
    // ANCHOR: get-token-metadata
    let tokenBalance = try await tokenIssuer.getIssuerTokenBalance()
    print("Token balance: {}", tokenBalance.balance)

    let tokenMetadata = try await tokenIssuer.getIssuerTokenMetadata()
    print("Token ticker: {}", tokenMetadata.ticker)
    // ANCHOR_END: get-token-metadata
    return tokenMetadata
}

func freezeToken(tokenIssuer: TokenIssuer) async throws {
    // ANCHOR: freeze-token
    let sparkAddress = "<spark address>"
    // Freeze the tokens held at the specified Spark address
    let freezeRequest = FreezeIssuerTokenRequest(
        address: sparkAddress
    )
    let freezeResponse = try await tokenIssuer.freezeIssuerToken(request: freezeRequest)

    // Unfreeze the tokens held at the specified Spark address
    let unfreezeRequest = UnfreezeIssuerTokenRequest(
        address: sparkAddress
    )
    let unfreezeResponse = try await tokenIssuer.unfreezeIssuerToken(request: unfreezeRequest)
    // ANCHOR_END: freeze-token
}
