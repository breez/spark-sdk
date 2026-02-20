import Foundation
import WebKit

// MARK: - Callback Type Aliases

/// Callback for enable requests.
/// Called when a website requests WebLN access.
///
/// - Parameter domain: The domain requesting access
/// - Returns: `true` to allow access, `false` to deny
public typealias OnEnableRequest = (String) async -> Bool

/// Callback for payment requests.
/// Called when a website requests to send a payment.
///
/// - Parameters:
///   - invoice: The BOLT11 invoice
///   - amountSats: The amount in satoshis
/// - Returns: `true` to approve payment, `false` to reject
public typealias OnPaymentRequest = (String, Int64) async -> Bool

/// Callback for LNURL requests.
/// Called when a website initiates an LNURL flow.
///
/// - Parameter request: The LNURL request details
/// - Returns: User's response with approval and optional amount/comment
public typealias OnLnurlRequest = (LnurlRequest) async -> LnurlUserResponse

// MARK: - Types

/// Represents the type of LNURL request.
public enum LnurlType {
    /// LNURL-pay request
    case pay
    /// LNURL-withdraw request
    case withdraw
    /// LNURL-auth request
    case auth
}

/// Represents an LNURL request that needs user approval.
/// Passed to the `onLnurlRequest` callback.
public struct LnurlRequest {
    /// The type of LNURL request
    public let type: LnurlType
    /// The domain of the LNURL service
    public let domain: String
    /// Minimum amount in sats (for pay/withdraw requests)
    public let minAmountSats: Int64?
    /// Maximum amount in sats (for pay/withdraw requests)
    public let maxAmountSats: Int64?
    /// LNURL metadata JSON string (for pay requests)
    public let metadata: String?
    /// Default description (for withdraw requests)
    public let defaultDescription: String?

    public init(
        type: LnurlType,
        domain: String,
        minAmountSats: Int64? = nil,
        maxAmountSats: Int64? = nil,
        metadata: String? = nil,
        defaultDescription: String? = nil
    ) {
        self.type = type
        self.domain = domain
        self.minAmountSats = minAmountSats
        self.maxAmountSats = maxAmountSats
        self.metadata = metadata
        self.defaultDescription = defaultDescription
    }
}

/// Represents the user's response to an LNURL request.
/// Returned from the `onLnurlRequest` callback.
public struct LnurlUserResponse {
    /// Whether the user approved the request
    public let approved: Bool
    /// Amount in sats selected by the user (for pay/withdraw)
    public let amountSats: Int64?
    /// Optional comment (for LNURL-pay)
    public let comment: String?

    public init(approved: Bool, amountSats: Int64? = nil, comment: String? = nil) {
        self.approved = approved
        self.amountSats = amountSats
        self.comment = comment
    }
}

/// WebLN error codes returned to JavaScript.
public enum WebLnErrorCode {
    public static let userRejected = "USER_REJECTED"
    public static let providerNotEnabled = "PROVIDER_NOT_ENABLED"
    public static let insufficientFunds = "INSUFFICIENT_FUNDS"
    public static let invalidParams = "INVALID_PARAMS"
    public static let unsupportedMethod = "UNSUPPORTED_METHOD"
    public static let internalError = "INTERNAL_ERROR"
}

// MARK: - WebLnController

/// Controller for WebLN support in iOS WKWebViews.
///
/// Injects the WebLN provider JavaScript into WebViews and handles
/// communication between the web page and the Breez SDK.
///
/// Usage:
/// ```swift
/// let controller = WebLnController(
///     sdk: sdk,
///     webView: webView,
///     onEnableRequest: { domain in
///         return await showEnableDialog(domain)
///     },
///     onPaymentRequest: { invoice, amountSats in
///         return await showPaymentDialog(invoice, amountSats)
///     },
///     onLnurlRequest: { request in
///         return await handleLnurlRequest(request)
///     }
/// )
/// controller.inject()
/// ```
public class WebLnController: NSObject, WKScriptMessageHandler {
    private let sdk: BreezSdk
    private weak var webView: WKWebView?
    private let onEnableRequest: OnEnableRequest
    private let onPaymentRequest: OnPaymentRequest
    private let onLnurlRequest: OnLnurlRequest

    private var enabledDomains = Set<String>()
    private var cachedPubkey: String?

    private static let supportedMethods = [
        "getInfo", "sendPayment", "makeInvoice",
        "signMessage", "verifyMessage", "lnurl"
    ]

    /// Creates a new WebLN controller.
    ///
    /// - Parameters:
    ///   - sdk: The Breez SDK instance
    ///   - webView: The WKWebView to inject WebLN into
    ///   - onEnableRequest: Callback when a site requests WebLN access
    ///   - onPaymentRequest: Callback when a site requests payment approval
    ///   - onLnurlRequest: Callback when a site initiates an LNURL flow
    public init(
        sdk: BreezSdk,
        webView: WKWebView,
        onEnableRequest: @escaping OnEnableRequest,
        onPaymentRequest: @escaping OnPaymentRequest,
        onLnurlRequest: @escaping OnLnurlRequest
    ) {
        self.sdk = sdk
        self.webView = webView
        self.onEnableRequest = onEnableRequest
        self.onPaymentRequest = onPaymentRequest
        self.onLnurlRequest = onLnurlRequest
        super.init()
    }

    /// Injects the WebLN provider script into the WebView
    public func inject() {
        guard let webView = webView else { return }

        // Add script message handler
        webView.configuration.userContentController.add(self, name: "BreezSparkWebLn")

        // Inject provider script
        let script = WKUserScript(
            source: weblnProviderScript,
            injectionTime: .atDocumentStart,
            forMainFrameOnly: false
        )
        webView.configuration.userContentController.addUserScript(script)
    }

    /// WKScriptMessageHandler implementation
    public func userContentController(
        _ userContentController: WKUserContentController,
        didReceive message: WKScriptMessage
    ) {
        guard let body = message.body as? String,
              let data = body.data(using: .utf8),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let id = json["id"] as? String,
              let method = json["method"] as? String else {
            return
        }

        let params = json["params"] as? [String: Any] ?? [:]

        Task {
            await handleMessage(id: id, method: method, params: params)
        }
    }

    private func handleMessage(id: String, method: String, params: [String: Any]) async {
        switch method {
        case "enable":
            await handleEnable(id: id, params: params)
        case "getInfo":
            await handleGetInfo(id: id)
        case "sendPayment":
            await handleSendPayment(id: id, params: params)
        case "makeInvoice":
            await handleMakeInvoice(id: id, params: params)
        case "signMessage":
            await handleSignMessage(id: id, params: params)
        case "verifyMessage":
            await handleVerifyMessage(id: id, params: params)
        case "lnurl":
            await handleLnurl(id: id, params: params)
        default:
            respond(id: id, error: "UNSUPPORTED_METHOD")
        }
    }

    private func handleEnable(id: String, params: [String: Any]) async {
        guard let domain = params["domain"] as? String else {
            respond(id: id, error: "INVALID_PARAMS")
            return
        }

        if enabledDomains.contains(domain) {
            respond(id: id, result: [:])
            return
        }

        let approved = await onEnableRequest(domain)
        if approved {
            enabledDomains.insert(domain)
            respond(id: id, result: [:])
        } else {
            respond(id: id, error: "USER_REJECTED")
        }
    }

    private func handleGetInfo(id: String) async {
        do {
            let pubkey = try await getNodePubkey()
            respond(id: id, result: [
                "node": ["pubkey": pubkey, "alias": ""],
                "methods": Self.supportedMethods
            ])
        } catch {
            respond(id: id, error: "INTERNAL_ERROR")
        }
    }

    private func getNodePubkey() async throws -> String {
        if let cached = cachedPubkey {
            return cached
        }

        let response = try await sdk.signMessage(
            request: SignMessageRequest(message: "webln_pubkey_request", compact: true)
        )
        cachedPubkey = response.pubkey
        return response.pubkey
    }

    private func handleSendPayment(id: String, params: [String: Any]) async {
        guard let paymentRequest = params["paymentRequest"] as? String else {
            respond(id: id, error: "INVALID_PARAMS")
            return
        }

        do {
            // Parse the invoice to get amount
            let parsed = try await sdk.parse(input: paymentRequest)
            var amountSats: Int64 = 0

            if case let .bolt11Invoice(details) = parsed {
                if let msat = details.amountMsat {
                    amountSats = Int64(msat / 1000)
                }
            }

            // Request payment confirmation from user
            let approved = await onPaymentRequest(paymentRequest, amountSats)
            guard approved else {
                respond(id: id, error: "USER_REJECTED")
                return
            }

            // Prepare and send payment
            let prepared = try await sdk.prepareSendPayment(
                request: PrepareSendPaymentRequest(paymentRequest: paymentRequest)
            )
            let result = try await sdk.sendPayment(
                request: SendPaymentRequest(
                    prepareResponse: prepared,
                    options: .bolt11Invoice(
                        preferSpark: false,
                        completionTimeoutSecs: 60
                    )
                )
            )

            // Extract preimage from payment details
            var preimage = ""
            if case .lightning(_, _, _, let htlcDetails, _, _, _) = result.payment.details {
                preimage = htlcDetails.preimage ?? ""
            }

            respond(id: id, result: ["preimage": preimage])
        } catch let error as SdkError {
            if case .InsufficientFunds = error {
                respond(id: id, error: "INSUFFICIENT_FUNDS")
            } else {
                respond(id: id, error: "INTERNAL_ERROR")
            }
        } catch {
            respond(id: id, error: "INTERNAL_ERROR")
        }
    }

    private func handleMakeInvoice(id: String, params: [String: Any]) async {
        do {
            let amount = (params["amount"] as? Int64) ?? (params["defaultAmount"] as? Int64)
            let memo = params["defaultMemo"] as? String ?? ""

            let response = try await sdk.receivePayment(
                request: ReceivePaymentRequest(
                    paymentMethod: .bolt11Invoice(
                        description: memo,
                        amountSats: amount.map { UInt64($0) },
                        expirySecs: nil,
                        paymentHash: nil
                    )
                )
            )

            respond(id: id, result: ["paymentRequest": response.paymentRequest])
        } catch {
            respond(id: id, error: "INTERNAL_ERROR")
        }
    }

    private func handleSignMessage(id: String, params: [String: Any]) async {
        guard let message = params["message"] as? String else {
            respond(id: id, error: "INVALID_PARAMS")
            return
        }

        do {
            let response = try await sdk.signMessage(
                request: SignMessageRequest(message: message, compact: true)
            )
            respond(id: id, result: [
                "message": message,
                "signature": response.signature
            ])
        } catch {
            respond(id: id, error: "INTERNAL_ERROR")
        }
    }

    private func handleVerifyMessage(id: String, params: [String: Any]) async {
        guard let signature = params["signature"] as? String,
              let message = params["message"] as? String else {
            respond(id: id, error: "INVALID_PARAMS")
            return
        }

        do {
            let pubkey = try await getNodePubkey()
            let response = try await sdk.checkMessage(
                request: CheckMessageRequest(
                    message: message,
                    pubkey: pubkey,
                    signature: signature
                )
            )

            if response.isValid {
                respond(id: id, result: [:])
            } else {
                respond(id: id, error: "INVALID_PARAMS")
            }
        } catch {
            respond(id: id, error: "INTERNAL_ERROR")
        }
    }

    private func handleLnurl(id: String, params: [String: Any]) async {
        guard let lnurlString = params["lnurl"] as? String else {
            respond(id: id, error: "INVALID_PARAMS")
            return
        }

        do {
            let parsed = try await sdk.parse(input: lnurlString)

            switch parsed {
            case let .lnurlPay(details):
                await handleLnurlPay(id: id, data: details)
            case let .lnurlWithdraw(details):
                await handleLnurlWithdraw(id: id, data: details)
            case let .lnurlAuth(details):
                await handleLnurlAuth(id: id, data: details)
            default:
                respond(id: id, error: "INVALID_PARAMS")
            }
        } catch {
            respond(id: id, error: "INTERNAL_ERROR")
        }
    }

    private func handleLnurlPay(id: String, data: LnurlPayRequestDetails) async {
        let lnurlResponse = await onLnurlRequest(LnurlRequest(
            type: .pay,
            domain: data.domain,
            minAmountSats: Int64(data.minSendable / 1000),
            maxAmountSats: Int64(data.maxSendable / 1000),
            metadata: data.metadataStr
        ))

        guard lnurlResponse.approved else {
            respond(id: id, error: "USER_REJECTED")
            return
        }

        do {
            let prepared = try await sdk.prepareLnurlPay(
                request: PrepareLnurlPayRequest(
                    amountSats: UInt64(lnurlResponse.amountSats ?? 0),
                    payRequest: data,
                    comment: lnurlResponse.comment
                )
            )

            let result = try await sdk.lnurlPay(
                request: LnurlPayRequest(prepareResponse: prepared)
            )

            // Extract preimage
            var preimage = ""
            if case .lightning(_, _, _, let htlcDetails, _, _, _) = result.payment.details {
                preimage = htlcDetails.preimage ?? ""
            }

            respond(id: id, result: ["status": "OK", "preimage": preimage])
        } catch let error as SdkError {
            if case .InsufficientFunds = error {
                respond(id: id, error: "INSUFFICIENT_FUNDS")
            } else {
                respond(id: id, error: "INTERNAL_ERROR")
            }
        } catch {
            respond(id: id, error: "INTERNAL_ERROR")
        }
    }

    private func handleLnurlWithdraw(id: String, data: LnurlWithdrawRequestDetails) async {
        let domain: String
        if let url = URL(string: data.callback) {
            domain = url.host ?? data.callback
        } else {
            domain = data.callback
        }

        let lnurlResponse = await onLnurlRequest(LnurlRequest(
            type: .withdraw,
            domain: domain,
            minAmountSats: Int64(data.minWithdrawable / 1000),
            maxAmountSats: Int64(data.maxWithdrawable / 1000),
            defaultDescription: data.defaultDescription
        ))

        guard lnurlResponse.approved else {
            respond(id: id, error: "USER_REJECTED")
            return
        }

        do {
            _ = try await sdk.lnurlWithdraw(
                request: LnurlWithdrawRequest(
                    amountSats: UInt64(lnurlResponse.amountSats ?? 0),
                    withdrawRequest: data
                )
            )
            respond(id: id, result: ["status": "OK"])
        } catch {
            respond(id: id, error: "INTERNAL_ERROR")
        }
    }

    private func handleLnurlAuth(id: String, data: LnurlAuthRequestDetails) async {
        let lnurlResponse = await onLnurlRequest(LnurlRequest(
            type: .auth,
            domain: data.domain
        ))

        guard lnurlResponse.approved else {
            respond(id: id, error: "USER_REJECTED")
            return
        }

        do {
            _ = try await sdk.lnurlAuth(requestData: data)
            respond(id: id, result: ["status": "OK"])
        } catch {
            respond(id: id, error: "INTERNAL_ERROR")
        }
    }

    private func respond(id: String, result: [String: Any]? = nil, error: String? = nil) {
        var response: [String: Any] = [
            "id": id,
            "success": error == nil
        ]
        if let result = result {
            response["result"] = result
        }
        if let error = error {
            response["error"] = error
        }

        guard let data = try? JSONSerialization.data(withJSONObject: response),
              let jsonString = String(data: data, encoding: .utf8) else {
            return
        }

        DispatchQueue.main.async { [weak self] in
            self?.webView?.evaluateJavaScript(
                "window.__breezSparkWebLnHandleResponse(\(jsonString));",
                completionHandler: nil
            )
        }
    }
}
