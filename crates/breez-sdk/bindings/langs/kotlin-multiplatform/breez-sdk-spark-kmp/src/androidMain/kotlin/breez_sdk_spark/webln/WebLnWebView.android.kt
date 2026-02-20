package breez_sdk_spark.webln

import android.annotation.SuppressLint
import android.graphics.Bitmap
import android.webkit.JavascriptInterface
import android.webkit.WebView
import android.webkit.WebViewClient

/**
 * Android implementation of WebLnWebView wrapping android.webkit.WebView.
 */
actual class WebLnWebView private constructor(
    private val webView: WebView
) {
    private val messageHandlers = mutableMapOf<String, MessageHandler>()

    actual val currentUrl: String?
        get() = webView.url

    @SuppressLint("SetJavaScriptEnabled")
    actual fun enableJavaScript() {
        webView.settings.javaScriptEnabled = true
    }

    actual fun evaluateJavaScript(script: String, callback: ((String?) -> Unit)?) {
        webView.evaluateJavascript(script) { result ->
            callback?.invoke(result)
        }
    }

    @SuppressLint("JavascriptInterface")
    actual fun addMessageHandler(name: String, onMessage: OnMessageReceived) {
        val handler = MessageHandler(onMessage)
        messageHandlers[name] = handler
        webView.addJavascriptInterface(handler, name)
    }

    actual fun removeMessageHandler(name: String) {
        messageHandlers.remove(name)
        webView.removeJavascriptInterface(name)
    }

    actual fun injectScript(script: String) {
        // Wrap the existing WebViewClient to re-inject the script on every page load.
        // On iOS, WKUserScript with atDocumentStart handles this automatically.
        // On Android, we need to intercept page loads via WebViewClient.
        val existingClient = webView.webViewClient
        webView.webViewClient = object : WebViewClient() {
            override fun onPageStarted(view: WebView, url: String?, favicon: Bitmap?) {
                existingClient.onPageStarted(view, url, favicon)
                view.evaluateJavascript(script, null)
            }

            override fun shouldOverrideUrlLoading(
                view: WebView,
                request: android.webkit.WebResourceRequest
            ): Boolean {
                return existingClient.shouldOverrideUrlLoading(view, request)
            }

            override fun onPageFinished(view: WebView, url: String?) {
                existingClient.onPageFinished(view, url)
            }

            override fun onReceivedError(
                view: WebView,
                request: android.webkit.WebResourceRequest,
                error: android.webkit.WebResourceError
            ) {
                existingClient.onReceivedError(view, request, error)
            }
        }
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
