import BreezSdkSpark

func signMessage(sdk: BreezSdk) async throws -> SignMessageResponse {
    // ANCHOR: sign-message
    // Set to true to get a compact signature rather than a DER
    let compact = true

    let signMessageRequest = SignMessageRequest(
        message: "<message to sign>",
        compact: compact
    )
    let signMessageResponse = try await sdk
        .signMessage(request: signMessageRequest)

    let signature = signMessageResponse.signature
    let pubkey = signMessageResponse.pubkey

    print("Pubkey: {}", pubkey);
    print("Signature: {}", signature);
    // ANCHOR_END: sign-message
    return signMessageResponse
}

func checkMessage(sdk: BreezSdk) async throws -> CheckMessageResponse {
    // ANCHOR: check-message
    let checkMessageRequest = CheckMessageRequest(
        message: "<message>",
        pubkey: "<pubkey of signer>",
        signature: "<message signature>"
    )
    let checkMessageResponse = try await sdk
        .checkMessage(request: checkMessageRequest)

    let isValid = checkMessageResponse.isValid

    print("Signature valid: {}", isValid);
    // ANCHOR_END: check-message
    return checkMessageResponse
}
