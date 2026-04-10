import BigNumber
import BreezSdkSpark
import Foundation

func preparePay(sdk: BreezSdk) async throws {
    // ANCHOR: prepare-lnurl-pay
    // Endpoint can also be of the form:
    // lnurlp://domain.com/lnurl-pay?key=val
    // lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excttsv9un7um9wdekjmmw84jxywf5x43rvv35xgmr2enrxanr2cfcvsmnwe3jxcukvde48qukgdec89snwde3vfjxvepjxpjnjvtpxd3kvdnxx5crxwpjvyunsephsz36jf
    let lnurlPayUrl = "lightning@address.com"

    let inputType = try await sdk.parse(input: lnurlPayUrl)
    if case .lightningAddress(v1: let details) = inputType {
        let amountSats = BInt(5_000)
        let optionalComment = "<comment>"
        let payRequest = details.payRequest
        let optionalValidateSuccessActionUrl = true

        let request = PrepareLnurlPayRequest(
            amount: amountSats,
            payRequest: payRequest,
            comment: optionalComment,
            validateSuccessActionUrl: optionalValidateSuccessActionUrl,
            tokenIdentifier: nil,
            conversionOptions: nil,
            feePolicy: nil
        )
        let prepareResponse = try await sdk.prepareLnurlPay(request: request)

        // If the fees are acceptable, continue to create the LNURL Pay
        let feeSats = prepareResponse.feeSats
        print("Fees: \(feeSats) sats")
    }

    // ANCHOR_END: prepare-lnurl-pay
}

func prepareLnurlPayFeesIncluded(sdk: BreezSdk, payRequest: LnurlPayRequestDetails) async throws {
    // ANCHOR: prepare-lnurl-pay-fees-included
    // By default (.feesExcluded), fees are added on top of the amount.
    // Use .feesIncluded to deduct fees from the amount instead.
    // The receiver gets amount minus fees.
    let amountSats = BInt(5_000)
    let optionalComment = "<comment>"
    let optionalValidateSuccessActionUrl = true

    let request = PrepareLnurlPayRequest(
        amount: amountSats,
        payRequest: payRequest,
        comment: optionalComment,
        validateSuccessActionUrl: optionalValidateSuccessActionUrl,
        tokenIdentifier: nil,
        conversionOptions: nil,
        feePolicy: .feesIncluded
    )
    let response = try await sdk.prepareLnurlPay(request: request)

    // If the fees are acceptable, continue to create the LNURL Pay
    let feeSats = response.feeSats
    print("Fees: \(feeSats) sats")
    // The receiver gets amountSats - feeSats
    // ANCHOR_END: prepare-lnurl-pay-fees-included
}

func pay(sdk: BreezSdk, prepareResponse: PrepareLnurlPayResponse) async throws {
    // ANCHOR: lnurl-pay
    let optionalIdempotencyKey = "<idempotency key uuid>"
    let response = try await sdk.lnurlPay(
        request: LnurlPayRequest(
            prepareResponse: prepareResponse,
            idempotencyKey: optionalIdempotencyKey
        ))
    // ANCHOR_END: lnurl-pay
    print("Response: \(response)")
}
