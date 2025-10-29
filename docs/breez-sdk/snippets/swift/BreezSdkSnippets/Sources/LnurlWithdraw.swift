import BreezSdkSpark
import Foundation

func withdraw(sdk: BreezSdk) async throws {
    // ANCHOR: lnurl-withdraw
    // Endpoint can also be of the form:
    // lnurlw://domain.com/lnurl-withdraw?key=val
    let lnurlWithdrawUrl = "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4exctthd96xserjv9mn7um9wdekjmmw843xxwpexdnxzen9vgunsvfexq6rvdecx93rgdmyxcuxverrvcursenpxvukzv3c8qunsdecx33nzwpnvg6ryc3hv93nzvecxgcxgwp3h33lxk"

    let inputType = try await sdk.parse(input: lnurlWithdrawUrl)
    if case .lnurlWithdraw(v1: let withdrawRequest) = inputType {
        // Amount to withdraw in sats between min/max withdrawable amounts
        let amountSats: UInt64 = 5_000
        let optionalCompletionTimeoutSecs: UInt32 = 30

        let request = LnurlWithdrawRequest(
            amountSats: amountSats,
            withdrawRequest: withdrawRequest,
            completionTimeoutSecs: optionalCompletionTimeoutSecs
        )
        let response = try await sdk.lnurlWithdraw(request: request)

        let payment = response.payment
        print("Payment: \(payment)")
    }
    // ANCHOR_END: lnurl-withdraw
}
