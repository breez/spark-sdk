import BreezSdkSpark
import Foundation

func preparePay(sdk: BreezSdk) async throws {
    // ANCHOR: prepare-lnurl-pay
    // Endpoint can also be of the form:
    // lnurlp://domain.com/lnurl-pay?key=val
    // lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43rvv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3kvdnxx5crxwpjvyunsephsz36jf
    let lnurlPayUrl = "lightning@address.com"

    if let inputType = try await parse(input: lnurlPayUrl) {
        if case .lightningAddress(let v1: details) = inputType {
            let amountSats = 5_000
            let optionalComment = "<comment>"
            let payRequest = details.payRequest
            let optionalValidateSuccessActionUrl = true

            let request = PrepareLnurlPayRequest(
                amountSats: amountSats,
                comment: optionalComment,
                payRequest: payRequest,
                validateSuccessActionUrl: optionalValidateSuccessActionUrl
            )
            response = try await sdk.prepareLnurlPay(request: request)

            // If the fees are acceptable, continue to create the LNURL Pay
            let feesSat = response.feeSats
            print("Fees: \(feesSat) sats")
        }
    }
    // ANCHOR_END: prepare-lnurl-pay
}

func pay(sdk: BreezSdk, prepareResponse: PrepareLnurlPayResponse) async throws {
    // ANCHOR: lnurl-pay
    let response = try await sdk.lnurlPay(
        request: LnurlPayRequest(
            prepareResponse: prepareResponse
        ))
    // ANCHOR_END: lnurl-pay
}
