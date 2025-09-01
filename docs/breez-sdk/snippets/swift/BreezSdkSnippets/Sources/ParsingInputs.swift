import BreezSdkSpark
import Foundation

func parseInput() async throws {
    // ANCHOR: parse-inputs
    let input = "an input to be parsed..."

    do {
        let inputType = try await parse(input: input)
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

        default:
            break  // Other input types are available
        }
    } catch {
        print("Failed to parse input: \(error)")
    }
    // ANCHOR_END: parse-inputs
}

