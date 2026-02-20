package breez_sdk_spark.webln

import kotlinx.cinterop.ExperimentalForeignApi
import platform.WebKit.WKScriptMessage
import platform.WebKit.WKScriptMessageHandlerProtocol
import platform.WebKit.WKUserContentController
import platform.WebKit.WKUserScript
import platform.WebKit.WKUserScriptInjectionTime
import platform.WebKit.WKWebView
import platform.darwin.NSObject

/**
 * iOS implementation of WebLnWebView wrapping WKWebView.
 */
@OptIn(ExperimentalForeignApi::class)
actual class WebLnWebView private constructor(
    private val webView: WKWebView
) {
    private val messageHandlers = mutableMapOf<String, MessageHandler>()

    actual val currentUrl: String?
        get() = webView.URL?.absoluteString

    actual fun enableJavaScript() {
        // WKWebView has JavaScript enabled by default
        // Configuration changes need to be done before WKWebView creation
        // Nothing to do here for WKWebView
    }

    actual fun evaluateJavaScript(script: String, callback: ((String?) -> Unit)?) {
        webView.evaluateJavaScript(script) { result, error ->
            if (callback != null) {
                if (error != null) {
                    callback(null)
                } else {
                    callback(result?.toString())
                }
            }
        }
    }

    actual fun addMessageHandler(name: String, onMessage: OnMessageReceived) {
        val handler = MessageHandler(onMessage)
        messageHandlers[name] = handler
        webView.configuration.userContentController.addScriptMessageHandler(handler, name)
    }

    actual fun removeMessageHandler(name: String) {
        messageHandlers.remove(name)
        webView.configuration.userContentController.removeScriptMessageHandlerForName(name)
    }

    /**
     * Injects a user script at document start.
     */
    actual fun injectScript(script: String) {
        val userScript = WKUserScript(
            source = script,
            injectionTime = WKUserScriptInjectionTime.WKUserScriptInjectionTimeAtDocumentStart,
            forMainFrameOnly = false
        )
        webView.configuration.userContentController.addUserScript(userScript)
    }

    /**
     * Internal class to handle WKScriptMessageHandler calls.
     */
    private class MessageHandler(
        private val onMessage: OnMessageReceived
    ) : NSObject(), WKScriptMessageHandlerProtocol {
        override fun userContentController(
            userContentController: WKUserContentController,
            didReceiveScriptMessage: WKScriptMessage
        ) {
            val body = didReceiveScriptMessage.body
            if (body is String) {
                onMessage(body)
            }
        }
    }

    companion object {
        /**
         * Creates a WebLnWebView wrapper from an iOS WKWebView.
         *
         * @param webView The iOS WKWebView to wrap
         * @return A WebLnWebView instance
         */
        fun fromIos(webView: WKWebView): WebLnWebView {
            return WebLnWebView(webView)
        }
    }
}
