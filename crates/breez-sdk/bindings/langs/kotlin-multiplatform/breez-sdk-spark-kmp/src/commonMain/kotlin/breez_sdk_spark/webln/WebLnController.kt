package breez_sdk_spark.webln

import breez_sdk_spark.BreezSdk
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/**
 * Callback for enable requests.
 * Called when a website requests WebLN access.
 *
 * @param domain The domain requesting access
 * @return true to allow access, false to deny
 */
typealias OnEnableRequest = suspend (domain: String) -> Boolean

/**
 * Callback for payment requests.
 * Called when a website requests to send a payment.
 *
 * @param invoice The BOLT11 invoice
 * @param amountSats The amount in satoshis
 * @return true to approve payment, false to reject
 */
typealias OnPaymentRequest = suspend (invoice: String, amountSats: Long) -> Boolean

/**
 * Callback for LNURL requests.
 * Called when a website initiates an LNURL flow.
 *
 * @param request The LNURL request details
 * @return User's response with approval and optional amount/comment
 */
typealias OnLnurlRequest = suspend (request: LnurlRequest) -> LnurlUserResponse

/**
 * Controller for WebLN support in WebViews.
 *
 * Injects the WebLN provider JavaScript into WebViews and handles
 * communication between the web page and the Breez SDK.
 *
 * Usage:
 * ```kotlin
 * val controller = WebLnController(
 *     sdk = breezSdk,
 *     webView = webView,  // Platform-specific WebView wrapper
 *     onEnableRequest = { domain -> showEnableDialog(domain) },
 *     onPaymentRequest = { invoice, amount -> showPaymentDialog(invoice, amount) },
 *     onLnurlRequest = { request -> showLnurlDialog(request) }
 * )
 * controller.inject()
 * ```
 *
 * @param sdk The Breez SDK instance
 * @param webView Platform-specific WebView wrapper
 * @param onEnableRequest Callback when a site requests WebLN access
 * @param onPaymentRequest Callback when a site requests payment approval
 * @param onLnurlRequest Callback when a site initiates an LNURL flow
 */
class WebLnController(
    sdk: BreezSdk,
    private val webView: WebLnWebView,
    onEnableRequest: OnEnableRequest,
    onPaymentRequest: OnPaymentRequest,
    onLnurlRequest: OnLnurlRequest
) {
    private val scope = CoroutineScope(Dispatchers.Main)

    private val handler = WebLnMessageHandler(
        sdk = sdk,
        onEnableRequest = onEnableRequest,
        onPaymentRequest = onPaymentRequest,
        onLnurlRequest = onLnurlRequest,
        respond = { id, result, error -> respond(id, result, error) }
    )

    /**
     * Injects the WebLN provider JavaScript into the WebView.
     * Call this after the WebView loads a page.
     */
    fun inject() {
        webView.enableJavaScript()
        webView.addMessageHandler(WebLnMessageHandler.HANDLER_NAME) { json ->
            scope.launch {
                try {
                    handler.handleMessage(json)
                } catch (e: Exception) {
                    println("WebLnController: Error handling request: ${e.message}")
                }
            }
        }

        // Inject the provider script
        webView.injectScript(loadWeblnProviderScript())
    }

    /**
     * Cleans up resources. Call when the WebView is destroyed.
     */
    fun dispose() {
        webView.removeMessageHandler(WebLnMessageHandler.HANDLER_NAME)
        handler.clearEnabledDomains()
    }

    private fun respond(id: String, result: kotlinx.serialization.json.JsonObject?, error: String?) {
        val responseJson = buildResponseJson(id, result, error)
        webView.evaluateJavaScript(
            "window.__breezSparkWebLnHandleResponse($responseJson);",
            null
        )
    }
}
