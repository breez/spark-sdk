import BreezSDKSpark
import Foundation

func parseLnurlAuth(sdk: BreezSdk) throws {
    // ANCHOR: parse-lnurl-auth
    // LNURL-auth URL from a service
    // Can be in the form:
    // - lnurl1... (bech32 encoded)
    // - https://service.com/lnurl-auth?tag=login&k1=...
    let lnurlAuthUrl = "lnurl1..."

    if case let .lnurlAuth(requestData) = try sdk.parse(input: lnurlAuthUrl) {
        print("Domain: \(requestData.domain)")
        print("Action: \(String(describing: requestData.action))")

        // Show domain to user and ask for confirmation
        // This is important for security
    }
    // ANCHOR_END: parse-lnurl-auth
}

func authenticate(sdk: BreezSdk, requestData: LnurlAuthRequestDetails) throws {
    // ANCHOR: lnurl-auth
    // Perform LNURL authentication
    let result = try sdk.lnurlAuth(requestData: requestData)

    switch result {
    case .ok:
        print("Authentication successful")
    case .errorStatus(let data):
        print("Authentication failed: \(data.reason)")
    }
    // ANCHOR_END: lnurl-auth
}
