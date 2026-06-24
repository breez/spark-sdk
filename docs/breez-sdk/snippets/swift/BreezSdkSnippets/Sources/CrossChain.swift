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

func getCrossChainReceiveRoutes(sdk: BreezSdk) async throws {
    // ANCHOR: cross-chain-get-receive-routes
    let routes = try await sdk.getCrossChainRoutes(
        filter: .receive(contractAddress: nil))

    for route in routes {
        print("Route via \(route.provider): \(route.chain)/\(route.asset) -> Spark")
    }
    // ANCHOR_END: cross-chain-get-receive-routes
}

func receivePaymentCrossChain(sdk: BreezSdk, route: CrossChainRoutePair) async throws {
    // ANCHOR: cross-chain-receive
    // amount is in source-asset base units (e.g. USDC base units when source is USDC)
    let amount = BInt(1_000_000)
    // Optionally set the destination Spark-side asset. nil = auto: active
    // stable-balance token if the route supports it, otherwise BTC.
    let optionalDestination: SparkAsset? = nil
    // Optionally set the maximum slippage in basis points (10 to 500)
    let optionalMaxSlippageBps: UInt32? = 100

    let response = try await sdk.receivePayment(
        request: ReceivePaymentRequest(
            paymentMethod: .crossChain(
                route: route,
                amount: amount,
                destination: optionalDestination,
                maxSlippageBps: optionalMaxSlippageBps
            )
        ))

    print("Share this deposit address with the sender: \(response.paymentRequest)")
    if let info = response.crossChainInfo {
        let denom = info.tokenIdentifier != nil ? "USDB" : "BTC"
        print("Sender deposits: \(info.depositAmount)")
        print("Receiver gets ~\(info.expectedReceivedAmount) \(denom)")
        print("Quote expires at: \(info.expiresAt)")
    }
    // ANCHOR_END: cross-chain-receive
}
