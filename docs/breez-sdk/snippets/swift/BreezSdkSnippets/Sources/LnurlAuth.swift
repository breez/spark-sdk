import BreezSdkSpark
import Foundation

func parseLnurlAuth(client: BreezClient) async throws {
    // ANCHOR: parse-lnurl-auth
    // LNURL-auth URL from a service
    // Can be in the form:
    // - lnurl1... (bech32 encoded)
    // - https://service.com/lnurl-auth?tag=login&k1=...
    let lnurlAuthUrl = "lnurl1..."

    if case .lnurlAuth(v1: let requestData) = try await client.parse(input: lnurlAuthUrl) {
        print("Domain: \(requestData.domain)")
        print("Action: \(String(describing: requestData.action))")

        // Show domain to user and ask for confirmation
        // This is important for security
    }
    // ANCHOR_END: parse-lnurl-auth
}

func authenticate(client: BreezClient, requestData: LnurlAuthRequestDetails) async throws {
    // ANCHOR: lnurl-auth
    // Perform LNURL authentication
    let result = try await client.lnurl().auth(requestData: requestData)

    switch result {
    case .ok:
        print("Authentication successful")
    case .errorStatus(errorDetails: let errorDetails):
        print("Authentication failed: \(errorDetails.reason)")
    }
    // ANCHOR_END: lnurl-auth
}
