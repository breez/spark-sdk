package breez_sdk_spark.webln

import breez_sdk_spark.*
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull

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
 * Represents the type of LNURL request.
 */
enum class LnurlType {
    /** LNURL-pay request */
    PAY,
    /** LNURL-withdraw request */
    WITHDRAW,
    /** LNURL-auth request */
    AUTH
}

/**
 * Represents an LNURL request that needs user approval.
 * Passed to the onLnurlRequest callback.
 */
data class LnurlRequest(
    /** The type of LNURL request */
    val type: LnurlType,
    /** The domain of the LNURL service */
    val domain: String,
    /** Minimum amount in sats (for pay/withdraw requests) */
    val minAmountSats: Long? = null,
    /** Maximum amount in sats (for pay/withdraw requests) */
    val maxAmountSats: Long? = null,
    /** LNURL metadata JSON string (for pay requests) */
    val metadata: String? = null,
    /** Default description (for withdraw requests) */
    val defaultDescription: String? = null
)

/**
 * Represents the user's response to an LNURL request.
 * Returned from the onLnurlRequest callback.
 */
data class LnurlUserResponse(
    /** Whether the user approved the request */
    val approved: Boolean,
    /** Amount in sats selected by the user (for pay/withdraw) */
    val amountSats: Long? = null,
    /** Optional comment (for LNURL-pay) */
    val comment: String? = null
)

/**
 * WebLN error codes returned to JavaScript.
 */
object WebLnErrorCode {
    const val USER_REJECTED = "USER_REJECTED"
    const val PROVIDER_NOT_ENABLED = "PROVIDER_NOT_ENABLED"
    const val INSUFFICIENT_FUNDS = "INSUFFICIENT_FUNDS"
    const val INVALID_PARAMS = "INVALID_PARAMS"
    const val UNSUPPORTED_METHOD = "UNSUPPORTED_METHOD"
    const val INTERNAL_ERROR = "INTERNAL_ERROR"
}

/**
 * Controller for WebLN support in Android WebViews.
 *
 * Injects the WebLN provider JavaScript into WebViews and handles
 * communication between the web page and the Breez SDK.
 *
 * Usage:
 * ```kotlin
 * val webView = WebLnWebView.fromAndroid(androidWebView)
 * val controller = WebLnController(
 *     sdk = sdk,
 *     webView = webView,
 *     onEnableRequest = { domain -> showEnableDialog(domain) },
 *     onPaymentRequest = { invoice, amountSats -> showPaymentDialog(invoice, amountSats) },
 *     onLnurlRequest = { request -> handleLnurlRequest(request) }
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
    private val sdk: BreezSdkInterface,
    private val webView: WebLnWebView,
    private val onEnableRequest: OnEnableRequest,
    private val onPaymentRequest: OnPaymentRequest,
    private val onLnurlRequest: OnLnurlRequest,
    private val scope: CoroutineScope = CoroutineScope(Dispatchers.Main)
) {
    private val enabledDomains = mutableSetOf<String>()
    private var cachedPubkey: String? = null
    private val json = Json { ignoreUnknownKeys = true }

    companion object {
        private val SUPPORTED_METHODS = listOf(
            "getInfo", "sendPayment", "makeInvoice",
            "signMessage", "verifyMessage", "lnurl"
        )
    }

    /**
     * Injects the WebLN provider script into the WebView
     */
    fun inject() {
        webView.enableJavaScript()
        webView.addMessageHandler("BreezSparkWebLn") { requestJson ->
            handleMessage(requestJson)
        }

        // Inject the provider script
        webView.injectScript(WEBLN_PROVIDER_SCRIPT)
    }

    /**
     * Cleans up resources. Call when the WebView is destroyed.
     */
    fun dispose() {
        webView.removeMessageHandler("BreezSparkWebLn")
        enabledDomains.clear()
    }

    /**
     * Handles incoming WebLN requests from JavaScript
     */
    private fun handleMessage(requestJson: String) {
        scope.launch {
            try {
                val request = json.parseToJsonElement(requestJson).jsonObject
                val id = request["id"]?.jsonPrimitive?.content ?: return@launch
                val method = request["method"]?.jsonPrimitive?.content ?: return@launch
                val params = request["params"]?.jsonObject ?: JsonObject(emptyMap())

                when (method) {
                    "enable" -> handleEnable(id, params)
                    "getInfo" -> handleGetInfo(id)
                    "sendPayment" -> handleSendPayment(id, params)
                    "makeInvoice" -> handleMakeInvoice(id, params)
                    "signMessage" -> handleSignMessage(id, params)
                    "verifyMessage" -> handleVerifyMessage(id, params)
                    "lnurl" -> handleLnurl(id, params)
                    else -> respond(id, error = "UNSUPPORTED_METHOD")
                }
            } catch (e: Exception) {
                android.util.Log.e("BreezSparkWebLn", "Error handling request", e)
            }
        }
    }

    private suspend fun handleEnable(id: String, params: JsonObject) {
        val domain = params["domain"]?.jsonPrimitive?.content
        if (domain == null) {
            respond(id, error = "INVALID_PARAMS")
            return
        }

        if (enabledDomains.contains(domain)) {
            respond(id, result = JsonObject(emptyMap()))
            return
        }

        val approved = onEnableRequest(domain)
        if (approved) {
            enabledDomains.add(domain)
            respond(id, result = JsonObject(emptyMap()))
        } else {
            respond(id, error = "USER_REJECTED")
        }
    }

    private suspend fun handleGetInfo(id: String) {
        try {
            val pubkey = getNodePubkey()
            respond(id, result = buildJsonObject {
                put("node", buildJsonObject {
                    put("pubkey", JsonPrimitive(pubkey))
                    put("alias", JsonPrimitive(""))
                })
                put("methods", Json.parseToJsonElement(json.encodeToString(SUPPORTED_METHODS)))
            })
        } catch (e: Exception) {
            respond(id, error = "INTERNAL_ERROR")
        }
    }

    private suspend fun getNodePubkey(): String {
        cachedPubkey?.let { return it }

        val response = sdk.signMessage(
            SignMessageRequest(message = "webln_pubkey_request", compact = true)
        )
        cachedPubkey = response.pubkey
        return response.pubkey
    }

    private suspend fun handleSendPayment(id: String, params: JsonObject) {
        val paymentRequest = params["paymentRequest"]?.jsonPrimitive?.content
        if (paymentRequest == null) {
            respond(id, error = "INVALID_PARAMS")
            return
        }

        try {
            // Parse the invoice to get amount
            val parsed = sdk.parse(paymentRequest)
            var amountSats = 0L

            if (parsed is InputType.Bolt11Invoice) {
                parsed.v1.amountMsat?.let { msat ->
                    amountSats = msat.toLong() / 1000
                }
            }

            // Request payment confirmation from user
            val approved = onPaymentRequest(paymentRequest, amountSats)
            if (!approved) {
                respond(id, error = "USER_REJECTED")
                return
            }

            // Prepare and send payment
            val prepared = sdk.prepareSendPayment(
                PrepareSendPaymentRequest(paymentRequest = paymentRequest)
            )
            val result = sdk.sendPayment(
                SendPaymentRequest(
                    prepareResponse = prepared,
                    options = SendPaymentOptions.Bolt11Invoice(
                        preferSpark = false,
                        completionTimeoutSecs = 60u,
                    ),
                )
            )

            // Extract preimage from payment details
            var preimage = ""
            when (val details = result.payment.details) {
                is PaymentDetails.Lightning -> {
                    preimage = details.htlcDetails.preimage ?: ""
                }
                else -> {}
            }

            respond(id, result = buildJsonObject {
                put("preimage", JsonPrimitive(preimage))
            })
        } catch (e: SdkException.InsufficientFunds) {
            respond(id, error = "INSUFFICIENT_FUNDS")
        } catch (e: Exception) {
            respond(id, error = "INTERNAL_ERROR")
        }
    }

    private suspend fun handleMakeInvoice(id: String, params: JsonObject) {
        try {
            val amount = params["amount"]?.jsonPrimitive?.longOrNull
                ?: params["defaultAmount"]?.jsonPrimitive?.longOrNull
            val memo = params["defaultMemo"]?.jsonPrimitive?.content ?: ""

            val response = sdk.receivePayment(
                ReceivePaymentRequest(
                    paymentMethod = ReceivePaymentMethod.Bolt11Invoice(
                        description = memo,
                        amountSats = amount?.toULong(),
                        expirySecs = null,
                        paymentHash = null
                    )
                )
            )

            respond(id, result = buildJsonObject {
                put("paymentRequest", JsonPrimitive(response.paymentRequest))
            })
        } catch (e: Exception) {
            respond(id, error = "INTERNAL_ERROR")
        }
    }

    private suspend fun handleSignMessage(id: String, params: JsonObject) {
        val message = params["message"]?.jsonPrimitive?.content
        if (message == null) {
            respond(id, error = "INVALID_PARAMS")
            return
        }

        try {
            val response = sdk.signMessage(
                SignMessageRequest(message = message, compact = true)
            )
            respond(id, result = buildJsonObject {
                put("message", JsonPrimitive(message))
                put("signature", JsonPrimitive(response.signature))
            })
        } catch (e: Exception) {
            respond(id, error = "INTERNAL_ERROR")
        }
    }

    private suspend fun handleVerifyMessage(id: String, params: JsonObject) {
        val signature = params["signature"]?.jsonPrimitive?.content
        val message = params["message"]?.jsonPrimitive?.content

        if (signature == null || message == null) {
            respond(id, error = "INVALID_PARAMS")
            return
        }

        try {
            val pubkey = getNodePubkey()
            val response = sdk.checkMessage(
                CheckMessageRequest(
                    message = message,
                    pubkey = pubkey,
                    signature = signature
                )
            )

            if (response.isValid) {
                respond(id, result = JsonObject(emptyMap()))
            } else {
                respond(id, error = "INVALID_PARAMS")
            }
        } catch (e: Exception) {
            respond(id, error = "INTERNAL_ERROR")
        }
    }

    private suspend fun handleLnurl(id: String, params: JsonObject) {
        val lnurlString = params["lnurl"]?.jsonPrimitive?.content
        if (lnurlString == null) {
            respond(id, error = "INVALID_PARAMS")
            return
        }

        try {
            when (val parsed = sdk.parse(lnurlString)) {
                is InputType.LnurlPay -> handleLnurlPay(id, parsed.v1)
                is InputType.LnurlWithdraw -> handleLnurlWithdraw(id, parsed.v1)
                is InputType.LnurlAuth -> handleLnurlAuth(id, parsed.v1)
                else -> respond(id, error = "INVALID_PARAMS")
            }
        } catch (e: Exception) {
            respond(id, error = "INTERNAL_ERROR")
        }
    }

    private suspend fun handleLnurlPay(id: String, data: LnurlPayRequestDetails) {
        val lnurlResponse = onLnurlRequest(
            LnurlRequest(
                type = LnurlType.PAY,
                domain = data.domain,
                minAmountSats = data.minSendable.toLong() / 1000,
                maxAmountSats = data.maxSendable.toLong() / 1000,
                metadata = data.metadataStr
            )
        )

        if (!lnurlResponse.approved) {
            respond(id, error = "USER_REJECTED")
            return
        }

        try {
            val prepared = sdk.prepareLnurlPay(
                PrepareLnurlPayRequest(
                    payRequest = data,
                    amountSats = (lnurlResponse.amountSats ?: 0).toULong(),
                    comment = lnurlResponse.comment
                )
            )

            val result = sdk.lnurlPay(
                LnurlPayRequest(prepareResponse = prepared)
            )

            // Extract preimage
            var preimage = ""
            when (val details = result.payment.details) {
                is PaymentDetails.Lightning -> {
                    preimage = details.htlcDetails.preimage ?: ""
                }
                else -> {}
            }

            respond(id, result = buildJsonObject {
                put("status", JsonPrimitive("OK"))
                put("preimage", JsonPrimitive(preimage))
            })
        } catch (e: SdkException.InsufficientFunds) {
            respond(id, error = "INSUFFICIENT_FUNDS")
        } catch (e: Exception) {
            respond(id, error = "INTERNAL_ERROR")
        }
    }

    private suspend fun handleLnurlWithdraw(id: String, data: LnurlWithdrawRequestDetails) {
        val domain = try {
            java.net.URI(data.callback).host
        } catch (e: Exception) {
            data.callback
        }

        val lnurlResponse = onLnurlRequest(
            LnurlRequest(
                type = LnurlType.WITHDRAW,
                domain = domain,
                minAmountSats = data.minWithdrawable.toLong() / 1000,
                maxAmountSats = data.maxWithdrawable.toLong() / 1000,
                defaultDescription = data.defaultDescription
            )
        )

        if (!lnurlResponse.approved) {
            respond(id, error = "USER_REJECTED")
            return
        }

        try {
            sdk.lnurlWithdraw(
                LnurlWithdrawRequest(
                    withdrawRequest = data,
                    amountSats = (lnurlResponse.amountSats ?: 0).toULong()
                )
            )
            respond(id, result = buildJsonObject {
                put("status", JsonPrimitive("OK"))
            })
        } catch (e: Exception) {
            respond(id, error = "INTERNAL_ERROR")
        }
    }

    private suspend fun handleLnurlAuth(id: String, data: LnurlAuthRequestDetails) {
        val lnurlResponse = onLnurlRequest(
            LnurlRequest(
                type = LnurlType.AUTH,
                domain = data.domain
            )
        )

        if (!lnurlResponse.approved) {
            respond(id, error = "USER_REJECTED")
            return
        }

        try {
            sdk.lnurlAuth(data)
            respond(id, result = buildJsonObject {
                put("status", JsonPrimitive("OK"))
            })
        } catch (e: Exception) {
            respond(id, error = "INTERNAL_ERROR")
        }
    }

    private fun respond(id: String, result: JsonObject? = null, error: String? = null) {
        val response = buildJsonObject {
            put("id", JsonPrimitive(id))
            put("success", JsonPrimitive(error == null))
            result?.let { put("result", it) }
            error?.let { put("error", JsonPrimitive(it)) }
        }

        val responseJson = json.encodeToString(response)
        webView.post {
            webView.evaluateJavaScript(
                "window.__breezSparkWebLnHandleResponse($responseJson);",
                null
            )
        }
    }
}
