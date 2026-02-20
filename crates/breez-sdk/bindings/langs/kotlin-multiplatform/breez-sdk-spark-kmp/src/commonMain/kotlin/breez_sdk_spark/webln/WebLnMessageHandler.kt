package breez_sdk_spark.webln

import breez_sdk_spark.*
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull

/**
 * Extracts the host/domain from a URL string using pure Kotlin.
 */
private val urlHostRegex = Regex("""^(?:\w+://)?([^/:]+)""")

private fun extractDomainFromUrl(url: String): String {
    return urlHostRegex.find(url)?.groupValues?.getOrNull(1) ?: url
}

/**
 * Shared WebLN message handler containing all common logic.
 * Platform-specific controllers delegate to this handler.
 */
internal class WebLnMessageHandler(
    private val sdk: BreezSdk,
    private val onEnableRequest: OnEnableRequest,
    private val onPaymentRequest: OnPaymentRequest,
    private val onLnurlRequest: OnLnurlRequest,
    private val respond: (id: String, result: JsonObject?, error: String?) -> Unit,
) {
    private val enabledDomains = mutableSetOf<String>()
    private var cachedPubkey: String? = null
    private val json = Json { ignoreUnknownKeys = true }

    companion object {
        const val HANDLER_NAME = "BreezSparkWebLn"
        val SUPPORTED_METHODS = listOf(
            "getInfo", "sendPayment", "makeInvoice",
            "signMessage", "verifyMessage", "lnurl"
        )
    }

    fun clearEnabledDomains() {
        enabledDomains.clear()
    }

    suspend fun handleMessage(requestJson: String) {
        val request = json.parseToJsonElement(requestJson).jsonObject
        val id = request["id"]?.jsonPrimitive?.content ?: return
        val method = request["method"]?.jsonPrimitive?.content ?: return
        val params = request["params"]?.jsonObject ?: JsonObject(emptyMap())

        when (method) {
            "enable" -> handleEnable(id, params)
            "getInfo" -> handleGetInfo(id)
            "sendPayment" -> handleSendPayment(id, params)
            "makeInvoice" -> handleMakeInvoice(id, params)
            "signMessage" -> handleSignMessage(id, params)
            "verifyMessage" -> handleVerifyMessage(id, params)
            "lnurl" -> handleLnurl(id, params)
            else -> respond(id, null, WebLnErrorCode.UNSUPPORTED_METHOD)
        }
    }

    private suspend fun handleEnable(id: String, params: JsonObject) {
        val domain = params["domain"]?.jsonPrimitive?.content
        if (domain == null) {
            respond(id, null, WebLnErrorCode.INVALID_PARAMS)
            return
        }

        if (enabledDomains.contains(domain)) {
            respond(id, JsonObject(emptyMap()), null)
            return
        }

        val approved = onEnableRequest(domain)
        if (approved) {
            enabledDomains.add(domain)
            respond(id, JsonObject(emptyMap()), null)
        } else {
            respond(id, null, WebLnErrorCode.USER_REJECTED)
        }
    }

    private suspend fun handleGetInfo(id: String) {
        try {
            val pubkey = getNodePubkey()
            respond(id, buildJsonObject {
                put("node", buildJsonObject {
                    put("pubkey", JsonPrimitive(pubkey))
                    put("alias", JsonPrimitive(""))
                })
                put("methods", Json.parseToJsonElement(json.encodeToString(SUPPORTED_METHODS)))
            }, null)
        } catch (e: Exception) {
            respond(id, null, WebLnErrorCode.INTERNAL_ERROR)
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
            respond(id, null, WebLnErrorCode.INVALID_PARAMS)
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
                respond(id, null, WebLnErrorCode.USER_REJECTED)
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
            val preimage = extractPreimage(result.payment.details)

            respond(id, buildJsonObject {
                put("preimage", JsonPrimitive(preimage))
            }, null)
        } catch (e: SdkException.InsufficientFunds) {
            respond(id, null, WebLnErrorCode.INSUFFICIENT_FUNDS)
        } catch (e: Exception) {
            respond(id, null, WebLnErrorCode.INTERNAL_ERROR)
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

            respond(id, buildJsonObject {
                put("paymentRequest", JsonPrimitive(response.paymentRequest))
            }, null)
        } catch (e: Exception) {
            respond(id, null, WebLnErrorCode.INTERNAL_ERROR)
        }
    }

    private suspend fun handleSignMessage(id: String, params: JsonObject) {
        val message = params["message"]?.jsonPrimitive?.content
        if (message == null) {
            respond(id, null, WebLnErrorCode.INVALID_PARAMS)
            return
        }

        try {
            val response = sdk.signMessage(
                SignMessageRequest(message = message, compact = true)
            )
            respond(id, buildJsonObject {
                put("message", JsonPrimitive(message))
                put("signature", JsonPrimitive(response.signature))
            }, null)
        } catch (e: Exception) {
            respond(id, null, WebLnErrorCode.INTERNAL_ERROR)
        }
    }

    private suspend fun handleVerifyMessage(id: String, params: JsonObject) {
        val signature = params["signature"]?.jsonPrimitive?.content
        val message = params["message"]?.jsonPrimitive?.content

        if (signature == null || message == null) {
            respond(id, null, WebLnErrorCode.INVALID_PARAMS)
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
                respond(id, JsonObject(emptyMap()), null)
            } else {
                respond(id, null, WebLnErrorCode.INVALID_PARAMS)
            }
        } catch (e: Exception) {
            respond(id, null, WebLnErrorCode.INTERNAL_ERROR)
        }
    }

    private suspend fun handleLnurl(id: String, params: JsonObject) {
        val lnurlString = params["lnurl"]?.jsonPrimitive?.content
        if (lnurlString == null) {
            respond(id, null, WebLnErrorCode.INVALID_PARAMS)
            return
        }

        try {
            when (val parsed = sdk.parse(lnurlString)) {
                is InputType.LnurlPay -> handleLnurlPay(id, parsed.v1)
                is InputType.LnurlWithdraw -> handleLnurlWithdraw(id, parsed.v1)
                is InputType.LnurlAuth -> handleLnurlAuth(id, parsed.v1)
                else -> respond(id, null, WebLnErrorCode.INVALID_PARAMS)
            }
        } catch (e: Exception) {
            respond(id, null, WebLnErrorCode.INTERNAL_ERROR)
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
            respond(id, null, WebLnErrorCode.USER_REJECTED)
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

            val preimage = extractPreimage(result.payment.details)

            respond(id, buildJsonObject {
                put("status", JsonPrimitive("OK"))
                put("preimage", JsonPrimitive(preimage))
            }, null)
        } catch (e: SdkException.InsufficientFunds) {
            respond(id, null, WebLnErrorCode.INSUFFICIENT_FUNDS)
        } catch (e: Exception) {
            respond(id, null, WebLnErrorCode.INTERNAL_ERROR)
        }
    }

    private suspend fun handleLnurlWithdraw(id: String, data: LnurlWithdrawRequestDetails) {
        val domain = extractDomainFromUrl(data.callback)

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
            respond(id, null, WebLnErrorCode.USER_REJECTED)
            return
        }

        try {
            sdk.lnurlWithdraw(
                LnurlWithdrawRequest(
                    withdrawRequest = data,
                    amountSats = (lnurlResponse.amountSats ?: 0).toULong()
                )
            )
            respond(id, buildJsonObject {
                put("status", JsonPrimitive("OK"))
            }, null)
        } catch (e: Exception) {
            respond(id, null, WebLnErrorCode.INTERNAL_ERROR)
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
            respond(id, null, WebLnErrorCode.USER_REJECTED)
            return
        }

        try {
            sdk.lnurlAuth(data)
            respond(id, buildJsonObject {
                put("status", JsonPrimitive("OK"))
            }, null)
        } catch (e: Exception) {
            respond(id, null, WebLnErrorCode.INTERNAL_ERROR)
        }
    }

    private fun extractPreimage(details: PaymentDetails?): String {
        return when (details) {
            is PaymentDetails.Lightning -> details.htlcDetails.preimage ?: ""
            else -> ""
        }
    }
}

/**
 * Builds a JSON response string for sending back to JavaScript.
 */
internal fun buildResponseJson(id: String, result: JsonObject?, error: String?): String {
    val json = Json { ignoreUnknownKeys = true }
    val response = buildJsonObject {
        put("id", JsonPrimitive(id))
        put("success", JsonPrimitive(error == null))
        result?.let { put("result", it) }
        error?.let { put("error", JsonPrimitive(it)) }
    }
    return json.encodeToString(response)
}
