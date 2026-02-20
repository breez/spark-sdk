package breez_sdk_spark.webln

/**
 * JVM stub implementation of WebLnWebView.
 *
 * WebLN support is only available on Android and iOS platforms.
 * This stub exists to satisfy KMP compilation requirements.
 */
actual class WebLnWebView private constructor() {
    actual val currentUrl: String?
        get() = throw UnsupportedOperationException("WebLN is not supported on JVM")

    actual fun enableJavaScript() {
        throw UnsupportedOperationException("WebLN is not supported on JVM")
    }

    actual fun evaluateJavaScript(script: String, callback: ((String?) -> Unit)?) {
        throw UnsupportedOperationException("WebLN is not supported on JVM")
    }

    actual fun addMessageHandler(name: String, onMessage: OnMessageReceived) {
        throw UnsupportedOperationException("WebLN is not supported on JVM")
    }

    actual fun removeMessageHandler(name: String) {
        throw UnsupportedOperationException("WebLN is not supported on JVM")
    }

    actual fun injectScript(script: String) {
        throw UnsupportedOperationException("WebLN is not supported on JVM")
    }
}
