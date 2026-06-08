package technology.breez.spark.passkey

import android.app.Activity
import breez_sdk_spark.PasskeyClient
import breez_sdk_spark.PasskeyConfig
import breez_sdk_spark.PasskeyProviderOptions
import breez_sdk_spark.PrfProvider

/**
 * Zero-config [PasskeyClient] wired to the built-in [PasskeyProvider].
 * Defaults to the Breez shared RP (`keys.breez.technology`), so a
 * Breez-registered app needs only its relay key; set `providerOptions`
 * on [config] to use your own RP.
 *
 * Takes an [activityProvider] because the platform Credential Manager
 * needs an `Activity` to present its UI. Apps that need a credential
 * registry or a custom PRF backend build the provider themselves and
 * inject it through [PasskeyClientBuilder].
 *
 * @param breezApiKey Breez relay key for authenticated (NIP-42) label
 *   storage. Pass `null` for public relays only.
 * @param activityProvider Called lazily on every ceremony to obtain the
 *   foreground [Activity] that drives Credential Manager.
 * @param config Passkey client config (`providerOptions` / `defaultLabel`).
 */
public fun PasskeyClient(
    breezApiKey: String? = null,
    activityProvider: () -> Activity,
    config: PasskeyConfig? = null,
): PasskeyClient {
    val provider = PasskeyProvider(
        activityProvider = activityProvider,
        options = config?.providerOptions ?: PasskeyProviderOptions(),
    )
    return PasskeyClient(provider, breezApiKey, config)
}

/**
 * Builder for a [PasskeyClient] backed by a caller-supplied
 * [PrfProvider]. Use this for a custom PRF backend (hardware key,
 * FIDO2, file-backed). For the zero-config case use the
 * [PasskeyClient] factory above, which takes the `activityProvider`.
 *
 * @param breezApiKey Breez relay key for authenticated (NIP-42) label
 *   storage. Pass `null` for public relays only.
 * @param config Passkey client config. `defaultLabel` applies as the
 *   label-store default; `providerOptions` is owned by the injected
 *   provider.
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
     * Construct the client. Requires a provider to have been injected via
     * [withPrfProvider]; throws otherwise. For the zero-config built-in
     * provider, use `PasskeyClient(breezApiKey, activityProvider, config)`
     * instead (the platform provider cannot be defaulted without an
     * `Activity`).
     */
    fun build(): PasskeyClient {
        val resolved = requireNotNull(provider) {
            "PasskeyClientBuilder requires a PrfProvider: call withPrfProvider(...) " +
                "before build(). For the built-in provider, use " +
                "PasskeyClient(breezApiKey, activityProvider, config)."
        }
        return PasskeyClient(resolved, breezApiKey, config)
    }
}
