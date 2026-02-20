package breez_sdk_spark.webln

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
