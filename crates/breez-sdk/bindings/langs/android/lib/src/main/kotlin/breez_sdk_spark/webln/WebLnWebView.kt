package breez_sdk_spark.webln

import android.annotation.SuppressLint
import android.graphics.Bitmap
import android.webkit.JavascriptInterface
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient

/**
 * Callback for messages received from JavaScript.
 */
typealias OnMessageReceived = (json: String) -> Unit

/**
 * WebView wrapper for WebLN support.
 *
 * This class wraps an Android WebView to provide a consistent API
 * for WebLN integration, matching the Kotlin Multiplatform interface.
 *
 * Usage:
 * ```kotlin
 * val webView: android.webkit.WebView = ...
 * val wrapper = WebLnWebView.fromAndroid(webView)
 * val controller = WebLnController(sdk, wrapper, ...)
 * controller.inject()
 * ```
 */
class WebLnWebView private constructor(
    private val webView: WebView
) {
    private val messageHandlers = mutableMapOf<String, MessageHandler>()

    /**
     * The current URL loaded in the WebView, or null if none.
     */
    val currentUrl: String?
        get() = webView.url

    /**
     * Enables JavaScript execution in the WebView.
     */
    @SuppressLint("SetJavaScriptEnabled")
    fun enableJavaScript() {
        webView.settings.javaScriptEnabled = true
    }

    /**
     * Evaluates JavaScript code in the WebView.
     *
     * @param script The JavaScript code to execute
     * @param callback Optional callback with the result
     */
    fun evaluateJavaScript(script: String, callback: ((String?) -> Unit)? = null) {
        webView.evaluateJavascript(script) { result ->
            callback?.invoke(result)
        }
    }

    /**
     * Adds a JavaScript interface/message handler for receiving messages from JS.
     *
     * @param name The name of the interface (e.g., "BreezSparkWebLn")
     * @param onMessage Callback invoked when JS sends a message
     */
    @SuppressLint("JavascriptInterface")
    fun addMessageHandler(name: String, onMessage: OnMessageReceived) {
        val handler = MessageHandler(onMessage)
        messageHandlers[name] = handler
        webView.addJavascriptInterface(handler, name)
    }

    /**
     * Removes a previously added message handler.
     *
     * @param name The name of the interface to remove
     */
    fun removeMessageHandler(name: String) {
        messageHandlers.remove(name)
        webView.removeJavascriptInterface(name)
    }

    /**
     * Injects a script to be executed on every page load.
     *
     * Wraps the existing WebViewClient to re-inject the script at page start,
     * matching the iOS behavior of WKUserScript with atDocumentStart injection.
     *
     * @param script The JavaScript code to inject
     */
    fun injectScript(script: String) {
        val existingClient = webView.webViewClient
        webView.webViewClient = object : WebViewClient() {
            override fun onPageStarted(view: WebView, url: String?, favicon: Bitmap?) {
                existingClient.onPageStarted(view, url, favicon)
                view.evaluateJavascript(script, null)
            }

            override fun shouldOverrideUrlLoading(
                view: WebView,
                request: WebResourceRequest
            ): Boolean {
                return existingClient.shouldOverrideUrlLoading(view, request)
            }

            override fun onPageFinished(view: WebView, url: String?) {
                existingClient.onPageFinished(view, url)
            }

            override fun onReceivedError(
                view: WebView,
                request: WebResourceRequest,
                error: WebResourceError
            ) {
                existingClient.onReceivedError(view, request, error)
            }
        }
    }

    /**
     * Posts a runnable to the WebView's message queue.
     *
     * @param action The action to run on the WebView's thread
     */
    fun post(action: Runnable) {
        webView.post(action)
    }

    /**
     * Internal class to handle JavaScript interface calls.
     */
    private class MessageHandler(private val onMessage: OnMessageReceived) {
        @JavascriptInterface
        fun postMessage(json: String) {
            onMessage(json)
        }
    }

    companion object {
        /**
         * Creates a WebLnWebView wrapper from an Android WebView.
         *
         * @param webView The Android WebView to wrap
         * @return A WebLnWebView instance
         */
        fun fromAndroid(webView: WebView): WebLnWebView {
            return WebLnWebView(webView)
        }
    }
}
