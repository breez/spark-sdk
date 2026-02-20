package breez_sdk_spark.webln

/**
 * Callback for messages received from JavaScript.
 */
typealias OnMessageReceived = (json: String) -> Unit

/**
 * Platform-agnostic WebView wrapper for WebLN support.
 *
 * This class abstracts the platform-specific WebView implementations
 * (Android WebView and iOS WKWebView) to provide a common interface
 * for the WebLN controller.
 *
 * ## Android Usage
 * ```kotlin
 * val webView: android.webkit.WebView = ...
 * val wrapper = WebLnWebView.fromAndroid(webView)
 * ```
 *
 * ## iOS Usage
 * ```kotlin
 * val webView: WKWebView = ...
 * val wrapper = WebLnWebView.fromIos(webView)
 * ```
 */
expect class WebLnWebView {
    /**
     * The current URL loaded in the WebView, or null if none.
     */
    val currentUrl: String?

    /**
     * Enables JavaScript execution in the WebView.
     */
    fun enableJavaScript()

    /**
     * Evaluates JavaScript code in the WebView.
     *
     * @param script The JavaScript code to execute
     * @param callback Optional callback with the result
     */
    fun evaluateJavaScript(script: String, callback: ((String?) -> Unit)? = null)

    /**
     * Adds a JavaScript interface/message handler for receiving messages from JS.
     *
     * @param name The name of the interface (e.g., "BreezSparkWebLn")
     * @param onMessage Callback invoked when JS sends a message
     */
    fun addMessageHandler(name: String, onMessage: OnMessageReceived)

    /**
     * Removes a previously added message handler.
     *
     * @param name The name of the interface to remove
     */
    fun removeMessageHandler(name: String)

    /**
     * Injects a script to be executed at document start.
     *
     * On iOS, this uses WKUserScript for proper document-start injection.
     * On Android, this evaluates the script immediately.
     *
     * @param script The JavaScript code to inject
     */
    fun injectScript(script: String)
}

/**
 * Returns the embedded WebLN provider JavaScript.
 *
 * @return The JavaScript code
 */
fun loadWeblnProviderScript(): String = WEBLN_PROVIDER_SCRIPT
