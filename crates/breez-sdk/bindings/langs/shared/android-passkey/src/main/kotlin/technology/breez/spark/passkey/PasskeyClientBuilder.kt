package technology.breez.spark.passkey

import breez_sdk_spark.PasskeyClient
import breez_sdk_spark.PasskeyConfig
import breez_sdk_spark.PrfProvider

/**
 * Builder for a [PasskeyClient] backed by a caller-supplied
 * [PrfProvider].
 *
 * Unlike the web and iOS clients, Android has no zero-config
 * convenience constructor: the built-in [PasskeyProvider] needs an
 * `Activity` (via its `activityProvider`) to drive Credential Manager,
 * so there is no provider the builder can default to. Always inject a
 * provider through [withPrfProvider] before calling [build].
 *
 * ```kotlin
 * val provider = PasskeyProvider(
 *     activityProvider = { activity },
 *     rpId = rpId,
 *     rpName = rpName,
 *     credentialRegistry = registry,
 * )
 * val client = PasskeyClientBuilder(breezApiKey = apiKey)
 *     .withPrfProvider(provider)
 *     .build()
 * ```
 *
 * @param breezApiKey Breez relay key for authenticated (NIP-42) label
 *   storage. Pass `null` for public relays only.
 * @param config Optional [PasskeyConfig] (e.g. a default label).
 */
class PasskeyClientBuilder(
    private val breezApiKey: String? = null,
    private val config: PasskeyConfig? = null,
) {
    private var provider: PrfProvider? = null

    /**
     * Inject the [PrfProvider] the client derives seeds through. The
     * built-in [PasskeyProvider] or any custom implementation is
     * accepted.
     */
    fun withPrfProvider(provider: PrfProvider): PasskeyClientBuilder = apply {
        this.provider = provider
    }

    /**
     * Construct the client. Requires a provider to have been injected
     * via [withPrfProvider]; throws otherwise (the platform provider
     * cannot be defaulted without an `Activity`).
     */
    fun build(): PasskeyClient {
        val resolved = requireNotNull(provider) {
            "PasskeyClientBuilder requires a PrfProvider on Android: the platform " +
                "PasskeyProvider needs an Activity, so there is no zero-config default. " +
                "Call withPrfProvider(...) before build()."
        }
        return PasskeyClient(resolved, breezApiKey, config)
    }
}
