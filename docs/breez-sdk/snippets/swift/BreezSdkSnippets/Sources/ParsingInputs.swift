import BreezSdkSpark
import Foundation

func parseInput(sdk: BreezSdk) async throws {
    // ANCHOR: parse-inputs
    let input = "an input to be parsed..."

    do {
        let inputType = try await sdk.parse(input: input)
        switch inputType {
        case .bitcoinAddress(v1: let details):
            print("Input is Bitcoin address \(details.address)")

        case .bolt11Invoice(v1: let details):
            let amount = details.amountMsat.map { String($0) } ?? "unknown"
            print("Input is BOLT11 invoice for \(amount) msats")

        case .lnurlPay(v1: let details):
            print(
                "Input is LNURL-Pay/Lightning address accepting min/max \(details.minSendable)/\(details.maxSendable) msats)"
            )
        case .lnurlWithdraw(v1: let details):
            print(
                "Input is LNURL-Withdraw for min/max \(details.minWithdrawable)/\(details.maxWithdrawable) msats"
            )

        case .sparkAddress(v1: let details):
            print("Input is Spark address \(details.address)")

        case .sparkInvoice(v1: let invoice):
            print("Input is Spark invoice:")
            if let tokenIdentifier = invoice.tokenIdentifier {
                print("  Amount: \(invoice.amount) base units of token with id \(tokenIdentifier)")
            } else {
                print("  Amount: \(invoice.amount) sats")
            }

            if let description = invoice.description {
                print("  Description: \(description)")
            }

            if let expiresAt = invoice.expiresAt {
                print("  Expires at: \(Date(timeIntervalSince1970: TimeInterval(expiresAt)))")
            }

            if let senderPublicKey = invoice.senderPublicKey {
                print("  Sender public key: \(senderPublicKey)")
            }

        default:
            break  // Other input types are available
        }
    } catch {
        print("Failed to parse input: \(error)")
    }
    // ANCHOR_END: parse-inputs
}

func setExternalInputParsers() async {
    // ANCHOR: set-external-input-parsers
    // Create the default config
    var config = defaultConfig(network: Network.mainnet)
    config.apiKey = "<breez api key>"

    // Configure external parsers
    config.externalInputParsers = [
        ExternalInputParser(
            providerId: "provider_a",
            inputRegex: "^provider_a",
            parserUrl: "https://parser-domain.com/parser?input=<input>"
        ),
        ExternalInputParser(
            providerId: "provider_b",
            inputRegex: "^provider_b",
            parserUrl: "https://parser-domain.com/parser?input=<input>"
        ),
    ]
    // ANCHOR_END: set-external-input-parsers
}
