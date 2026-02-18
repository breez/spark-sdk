import BigNumber
import BreezSdkSpark
import CryptoKit
import Foundation

extension Data {
    init?(hexString: String) {
        let scanner = Scanner(string: hexString)
        var data = Data()
        while !scanner.isAtEnd {
            var byte: UInt64 = 0
            if scanner.scanHexInt64(&byte) {
                data.append(UInt8(byte))
            } else {
                return nil
            }
        }
        self = data
    }

    func hexEncodedString() -> String {
        return self.map { String(format: "%02hhx", $0) }.joined()
    }
}

func sendHtlcPayment(sdk: BreezSdk) async throws -> Payment {
    // ANCHOR: send-htlc-payment
    let paymentRequest = "<spark address>"
    // Set the amount you wish to pay the receiver
    let amountSats = BInt(50_000)
    let prepareRequest = PrepareSendPaymentRequest(
        paymentRequest: paymentRequest,
        amount: amountSats,
        tokenIdentifier: nil,
        conversionOptions: nil,
        feePolicy: nil
    )
    let prepareResponse = try await sdk.prepareSendPayment(request: prepareRequest)

    // If the fees are acceptable, continue to create the HTLC Payment
    if case let .sparkAddress(_, fee, _) = prepareResponse.paymentMethod {
        print("Fees: \(fee) sats")
    }

    let preimage = "<32-byte unique preimage hex>"
    let preimageData = Data(hexString: preimage)!
    let paymentHashDigest = SHA256.hash(data: preimageData)
    let paymentHash = Data(paymentHashDigest).hexEncodedString()

    // Set the HTLC options
    let htlcOptions = SparkHtlcOptions(
        paymentHash: paymentHash,
        expiryDurationSecs: 1000
    )
    let options = SendPaymentOptions.sparkAddress(htlcOptions: htlcOptions)

    let request = SendPaymentRequest(
        prepareResponse: prepareResponse,
        options: options
    )
    let sendResponse = try await sdk.sendPayment(request: request)
    let payment = sendResponse.payment
    // ANCHOR_END: send-htlc-payment
    return payment
}

func receiveHodlInvoicePayment(sdk: BreezSdk) async throws {
    // ANCHOR: receive-hodl-invoice-payment
    let preimage = "<32-byte unique preimage hex>"
    let preimageData = Data(hexString: preimage)!
    let paymentHashDigest = SHA256.hash(data: preimageData)
    let paymentHash = Data(paymentHashDigest).hexEncodedString()

    let response = try await sdk.receivePayment(
        request: ReceivePaymentRequest(
            paymentMethod: ReceivePaymentMethod.bolt11Invoice(
                description: "HODL invoice",
                amountSats: 50_000,
                expirySecs: nil,
                paymentHash: paymentHash
            )
        )
    )

    let invoice = response.paymentRequest
    print("HODL invoice: \(invoice)")
    // ANCHOR_END: receive-hodl-invoice-payment
}

func listClaimableHtlcPayments(sdk: BreezSdk) async throws -> [Payment] {
    // ANCHOR: list-claimable-htlc-payments
    let request = ListPaymentsRequest(
        typeFilter: [PaymentType.receive],
        statusFilter: [PaymentStatus.pending],
        paymentDetailsFilter: [
            PaymentDetailsFilter.spark(
                htlcStatus: [SparkHtlcStatus.waitingForPreimage],
                conversionRefundNeeded: nil
            ),
            PaymentDetailsFilter.lightning(
                htlcStatus: [SparkHtlcStatus.waitingForPreimage]
            ),
        ]
    )

    let response = try await sdk.listPayments(request: request)
    let payments = response.payments

    for payment in payments {
        if case let .spark(_, htlcDetails, _) = payment.details, let htlc = htlcDetails {
            print("Spark HTLC expiry time: \(htlc.expiryTime)")
        } else if case let .lightning(_, _, _, htlcDetails, _, _, _) = payment.details {
            print("Lightning HTLC expiry time: \(htlcDetails.expiryTime)")
        }
    }
    // ANCHOR_END: list-claimable-htlc-payments
    return payments
}

func claimHtlcPayment(sdk: BreezSdk) async throws -> Payment {
    // ANCHOR: claim-htlc-payment
    let preimage = "<preimage hex>"
    let response = try await sdk.claimHtlcPayment(
        request: ClaimHtlcPaymentRequest(preimage: preimage)
    )
    let payment = response.payment
    // ANCHOR_END: claim-htlc-payment
    return payment
}
