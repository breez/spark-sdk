import BigNumber
import BreezSdkSpark
import Foundation

func getCrossChainRoutes(sdk: BreezSdk) async throws {
    // ANCHOR: cross-chain-get-routes
    let input = "<recipient address>"
    let parsed = try await sdk.parse(input: input)
    guard case let .crossChainAddress(v1: addressDetails) = parsed else {
        throw NSError(domain: "CrossChain", code: 1)
    }

    let routes = try await sdk.getCrossChainRoutes(
        filter: .send(addressDetails: addressDetails))

    for route in routes {
        print("Route via \(route.provider): \(route.chain)/\(route.asset)")
    }
    // ANCHOR_END: cross-chain-get-routes
}

func prepareSendPaymentCrossChain(
    sdk: BreezSdk,
    addressDetails: CrossChainAddressDetails,
    route: CrossChainRoutePair
) async throws {
    // ANCHOR: cross-chain-prepare
    // Optionally set the maximum slippage in basis points (10 to 500)
    let optionalMaxSlippageBps: UInt32? = 100

    let prepareResponse = try await sdk.prepareSendPayment(
        request: PrepareSendPaymentRequest(
            paymentRequest: .crossChain(
                address: addressDetails.address,
                route: route,
                maxSlippageBps: optionalMaxSlippageBps,
                targetOverpayBps: nil
            ),
            amount: BInt(50_000),
            tokenIdentifier: nil,
            conversionOptions: nil,
            feePolicy: nil
        ))

    if case let .crossChainAddress(
        _, _, amountIn, _, estimatedOut, feeAmount, _, _, _, _, expiresAt, _
    ) = prepareResponse.paymentMethod {
        print("Amount in: \(amountIn)")
        print("Estimated out: \(estimatedOut)")
        print("Provider fee: \(feeAmount)")
        print("Quote expires at: \(expiresAt)")
    }
    // ANCHOR_END: cross-chain-prepare
}

func sendPaymentCrossChain(sdk: BreezSdk, prepareResponse: PrepareSendPaymentResponse) async throws
{
    // ANCHOR: cross-chain-send
    // Only valid for sends with no token leg (see Retry safety).
    let optionalIdempotencyKey = "<idempotency key uuid>"
    let sendResponse = try await sdk.sendPayment(
        request: SendPaymentRequest(
            prepareResponse: prepareResponse,
            options: nil,
            idempotencyKey: optionalIdempotencyKey
        ))
    let payment = sendResponse.payment
    print(payment)
    // ANCHOR_END: cross-chain-send
}
