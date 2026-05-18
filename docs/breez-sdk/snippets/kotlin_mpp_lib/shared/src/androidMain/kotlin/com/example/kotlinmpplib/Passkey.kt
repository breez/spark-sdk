package com.example.kotlinmpplib

import android.app.Activity
import breez_sdk_spark.*
import technology.breez.spark.passkey.PasskeyProvider

// ANCHOR: implement-prf-provider
// Implement the PrfProvider interface for custom logic if the built-in
// PasskeyProvider doesn't fit your needs. Three required methods:
// deriveSeeds for derivation, isSupported for the capability probe;
// createPasskey for registration is optional.
class CustomPrfProvider : PrfProvider {
    override suspend fun deriveSeeds(request: DeriveSeedsRequest): List<ByteArray> {
        // Call platform passkey API with PRF extension. Use the dual-salt
        // ceremony when the authenticator supports it (one OS prompt for
        // N salts) and fall back to per-salt assertions otherwise.
        // Returns one 32-byte PRF output per salt in input order.
        TODO("Implement using WebAuthn or native passkey APIs")
    }

    override suspend fun isSupported(): Boolean {
        TODO("Check platform passkey availability")
    }

    override suspend fun createPasskey(excludeCredentialIds: List<ByteArray>): RegisteredCredential {
        // Register a new credential and return its ID, the WebAuthn
        // user.id the platform recorded (returned for host-side
        // correlation, never host-supplied), AAGUID, and BE flag.
        TODO("Implement registration via native passkey API")
    }

    override suspend fun checkDomainAssociation(): DomainAssociation {
        return DomainAssociation.Skipped("CustomPrfProvider does not verify domain association")
    }
}
// ANCHOR_END: implement-prf-provider

class PasskeySnippets(private val activity: Activity) {
    suspend fun checkAvailability() {
        // ANCHOR: check-availability
        // rpId is required. Pass your app's domain, or
        // PasskeyProvider.BREEZ_RP_ID if your app is Breez-registered.
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "my-app.com",
        )
        val passkey = PasskeyClient(prfProvider, null, null)

        when (val availability = passkey.checkAvailability()) {
            is PasskeyAvailability.Available -> Unit
            is PasskeyAvailability.PrfUnsupported -> Unit
            is PasskeyAvailability.NotAssociated -> {
                // Log.e("Breez", "Domain association failed (source=${availability.source}): ${availability.reason}")
            }
            is PasskeyAvailability.Skipped -> Unit
        }
        // ANCHOR_END: check-availability
    }

    suspend fun connectWithPasskey(): BreezSdk {
        // ANCHOR: connect-with-passkey
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "my-app.com",
        )
        val passkey = PasskeyClient(prfProvider, null, null)

        val response = passkey.signIn(SignInRequest(label = "personal"))

        val config = defaultConfig(Network.MAINNET)
        val sdk = connect(ConnectRequest(config, response.wallet.seed, "./.data"))
        // ANCHOR_END: connect-with-passkey
        return sdk
    }

    suspend fun registerNewPasskey(): BreezSdk {
        // ANCHOR: register-passkey
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "my-app.com",
        )
        val passkey = PasskeyClient(prfProvider, null, null)

        val response = passkey.register(RegisterRequest(label = "personal"))

        // Hosts SHOULD persist credential.credentialId (for excludeCredentialIds
        // bookkeeping) and credential.userId (for server-side correlation).
        // The SDK generates userId; it is never host-supplied.
        val persistedCredentialId = response.credential.credentialId
        val persistedUserId = response.credential.userId

        val config = defaultConfig(Network.MAINNET)
        val sdk = connect(ConnectRequest(config, response.wallet.seed, "./.data"))
        // ANCHOR_END: register-passkey
        return sdk
    }

    suspend fun listLabels(): List<String> {
        // ANCHOR: list-labels
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "my-app.com",
        )
        val config = PasskeyConfig(
            // Optional: override the default wallet label used when
            // register / signIn receive `label = null`. Falls back to the
            // SDK's internal "Default" when unset.
            defaultLabel = "personal",
        )
        // breezApiKey enables authenticated (NIP-42) Breez relay access
        // for label sync; pass null for public-relay-only.
        val passkey = PasskeyClient(prfProvider, "<breez api key>", config)

        val labels = passkey.labels().list()
        for (label in labels) {
            // Log.v("Breez", "Found label: $label")
        }
        // ANCHOR_END: list-labels
        return labels
    }

    suspend fun storeLabel() {
        // ANCHOR: store-label
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "my-app.com",
        )
        val passkey = PasskeyClient(prfProvider, "<breez api key>", null)

        passkey.labels().store("personal")
        // ANCHOR_END: store-label
    }

    suspend fun singleCtaOnboarding(): Wallet {
        // ANCHOR: signin-fallback-register
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "my-app.com",
        )
        val passkey = PasskeyClient(prfProvider, null, null)

        return try {
            val response = passkey.signIn(SignInRequest(label = null))
            response.wallet
        } catch (e: PrfProviderException.CredentialNotFound) {
            val response = passkey.register(RegisterRequest(label = "personal"))
            response.wallet
        }
        // ANCHOR_END: signin-fallback-register
    }

    suspend fun checkDomain() {
        // ANCHOR: domain-association
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "my-app.com",
        )
        val result = prfProvider.checkDomainAssociation()

        when (result) {
            is DomainAssociation.Associated -> Unit
            is DomainAssociation.NotAssociated -> {
                // Log.e("Breez", "Domain association failed (source=${result.source}): ${result.reason}")
            }
            is DomainAssociation.Skipped -> Unit
        }
        // ANCHOR_END: domain-association
    }

    suspend fun recoverFromAlreadyExists(): Wallet {
        // ANCHOR: recover-already-exists
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "my-app.com",
        )
        val passkey = PasskeyClient(prfProvider, null, null)

        return try {
            val response = passkey.register(
                RegisterRequest(
                    label = "personal",
                    excludeCredentialIds = emptyList(), // app-persisted credential IDs from prior registrations
                )
            )
            response.wallet
        } catch (e: PrfProviderException.CredentialAlreadyExists) {
            val response = passkey.signIn(SignInRequest(label = "personal"))
            response.wallet
        }
        // ANCHOR_END: recover-already-exists
    }

    suspend fun handleTimeout(): SignInResponse {
        // ANCHOR: handle-timeout
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "my-app.com",
        )
        val passkey = PasskeyClient(prfProvider, null, null)

        return try {
            passkey.signIn(SignInRequest(label = "personal"))
        } catch (e: PrfProviderException.UserTimedOut) {
            // Log.v("Breez", "Sign-in timed out: show \"Try Again\" UI.")
            throw e
        }
        // ANCHOR_END: handle-timeout
    }
}
