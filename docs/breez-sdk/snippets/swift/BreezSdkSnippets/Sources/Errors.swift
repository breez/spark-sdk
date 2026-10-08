import BreezSdkSpark

func handleErrors(sdk: BreezSdk, request: PrepareSendPaymentRequest) async {
    // ANCHOR: handle-errors
    do {
        let prepareResponse = try await sdk.prepareSendPayment(request: request)
        print("Payment prepared: \(prepareResponse.paymentMethod)")
    } catch SdkError.InsufficientFunds {
        print("Not enough funds for this payment")
    } catch SdkError.CrossChainDisabled(let docsUrl) {
        print("Cross-chain payments are not enabled, see \(docsUrl)")
    } catch {
        print("Failed to prepare the payment: \(error)")
    }
    // ANCHOR_END: handle-errors
}
