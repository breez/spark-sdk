import BreezSdkSpark
import WebKit

// ANCHOR: webln-integration
func setupWebLn(sdk: BreezSdk, webView: WKWebView) {
    // Create the WebLN controller with the SDK and WebView
    let controller = WebLnController(
        sdk: sdk,
        webView: webView,
        onEnableRequest: { domain in
            // Show a dialog asking the user to approve WebLN access
            // Return true to allow, false to deny
            await showEnableDialog(domain: domain)
        },
        onPaymentRequest: { invoice, amountSats in
            // Show a dialog asking the user to approve the payment
            // Return true to approve, false to reject
            await showPaymentDialog(invoice: invoice, amountSats: amountSats)
        },
        onLnurlRequest: { request in
            // Handle LNURL requests (pay, withdraw, auth)
            switch request.type {
            case .pay:
                // Show UI to select amount within min/max bounds
                // Return LnurlUserResponse with approved, amountSats, and optional comment
                return LnurlUserResponse(approved: true, amountSats: 1000)
            case .withdraw:
                // Show UI to select amount within min/max bounds
                return LnurlUserResponse(approved: true, amountSats: 1000)
            case .auth:
                // Show confirmation dialog
                return LnurlUserResponse(approved: true)
            }
        }
    )

    // Inject the WebLN provider into the WebView
    controller.inject()
}
// ANCHOR_END: webln-integration

// Placeholder functions - implement these with your UI framework
func showEnableDialog(domain: String) async -> Bool {
    // Show your own permission dialog here
    return true
}

func showPaymentDialog(invoice: String, amountSats: Int64) async -> Bool {
    // Show your own payment confirmation dialog here
    return true
}
