package com.example.kotlinmpplib

import android.app.Activity
import breez_sdk_spark.*
import technology.breez.spark.passkey.PasskeyProvider
import technology.breez.spark.passkey.createPasskeyClient
import technology.breez.spark.passkey.core.CredentialRegistry

// Stub for the snippet to compile. Use the BlockStoreCredentialRegistry
// reference impl from the passkey guide in production.
class BlockStoreCredentialRegistry : CredentialRegistry {
    override suspend fun read(rpId: String): List<ByteArray> = emptyList()
    override suspend fun add(rpId: String, credentialId: ByteArray) {}
    override suspend fun remove(rpId: String, credentialId: ByteArray) {}
    override suspend fun clear(rpId: String) {}
}

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
        // Pass `PasskeyProvider.BREEZ_RP_ID` instead of "my-app.com" if your
        // app is Breez-registered (shares credentials with other Breez apps).
        val config = defaultConfig(Network.MAINNET).apply { apiKey = "<breez api key>" }
        val passkey = createPasskeyClient(
            activityProvider = { activity },
            rpId = "my-app.com",
            rpName = "My App",
            sdkConfig = config,
        )

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
        // Single-CTA onboarding: silent sign-in, fall through to register.
        val config = defaultConfig(Network.MAINNET).apply { apiKey = "<breez api key>" }
        val passkey = createPasskeyClient(
            activityProvider = { activity },
            rpId = "my-app.com",
            rpName = "My App",
            sdkConfig = config,
        )

        val response = passkey.connectWithPasskey(
            ConnectWithPasskeyRequest(label = "personal")
        )

        // `registeredCredential` is the path discriminator (null on sign-in).
        response.registeredCredential?.let { credential ->
            val persistedId = credential.credentialId
        }

        val sdk = connect(ConnectRequest(config, response.wallet.seed, "./.data"))
        // ANCHOR_END: connect-with-passkey
        return sdk
    }

    suspend fun signInExistingUser(): SignInResponse {
        // ANCHOR: sign-in
        // Returning-user-only sign-in. No fall-through to register: use
        // `connectWithPasskey` when you also want the new-user path.
        val config = defaultConfig(Network.MAINNET).apply { apiKey = "<breez api key>" }
        val passkey = createPasskeyClient(
            activityProvider = { activity },
            rpId = "my-app.com",
            rpName = "My App",
            sdkConfig = config,
        )

        return passkey.signIn(SignInRequest(label = "personal"))
        // ANCHOR_END: sign-in
    }

    suspend fun registerNewPasskey(): BreezSdk {
        // ANCHOR: register-passkey
        val config = defaultConfig(Network.MAINNET).apply { apiKey = "<breez api key>" }
        val passkey = createPasskeyClient(
            activityProvider = { activity },
            rpId = "my-app.com",
            rpName = "My App",
            sdkConfig = config,
        )

        val response = passkey.register(RegisterRequest(label = "personal"))

        // Persist credentialId for future excludeCredentialIds.
        val persistedCredentialId = response.credential.credentialId
        val persistedUserId = response.credential.userId

        val sdk = connect(ConnectRequest(config, response.wallet.seed, "./.data"))
        // ANCHOR_END: register-passkey
        return sdk
    }

    suspend fun listLabels(): List<String> {
        // ANCHOR: list-labels
        val sdkConfig = defaultConfig(Network.MAINNET).apply { apiKey = "<breez api key>" }
        val passkey = createPasskeyClient(
            activityProvider = { activity },
            rpId = "my-app.com",
            rpName = "My App",
            sdkConfig = sdkConfig,
            // Default wallet label when register / signIn receive no label.
            passkeyConfig = PasskeyConfig(defaultLabel = "personal"),
        )

        val labels = passkey.labels().list()
        for (label in labels) {
            // Log.v("Breez", "Found label: $label")
        }
        // ANCHOR_END: list-labels
        return labels
    }

    suspend fun storeLabel() {
        // ANCHOR: store-label
        val config = defaultConfig(Network.MAINNET).apply { apiKey = "<breez api key>" }
        val passkey = createPasskeyClient(
            activityProvider = { activity },
            rpId = "my-app.com",
            rpName = "My App",
            sdkConfig = config,
        )

        passkey.labels().store("personal")
        // ANCHOR_END: store-label
    }

    suspend fun checkDomain() {
        // ANCHOR: domain-association
        // Lower-level diagnostic on the provider itself. Most hosts
        // can reach this through `passkey.checkAvailability()`, which
        // folds PRF support and domain association into a single call
        // (see the `check-availability` snippet above).
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "my-app.com",
            rpName = "My App",
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
        val config = defaultConfig(Network.MAINNET).apply { apiKey = "<breez api key>" }
        val passkey = createPasskeyClient(
            activityProvider = { activity },
            rpId = "my-app.com",
            rpName = "My App",
            sdkConfig = config,
        )

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
        val config = defaultConfig(Network.MAINNET).apply { apiKey = "<breez api key>" }
        val passkey = createPasskeyClient(
            activityProvider = { activity },
            rpId = "my-app.com",
            rpName = "My App",
            sdkConfig = config,
        )

        return try {
            passkey.signIn(SignInRequest(label = "personal"))
        } catch (e: PrfProviderException.UserTimedOut) {
            // Log.v("Breez", "Sign-in timed out: show \"Try Again\" UI.")
            throw e
        }
        // ANCHOR_END: handle-timeout
    }

    suspend fun withCredentialRegistry() {
        // ANCHOR: with-credential-registry
        // Opt-in CredentialRegistry. The SDK auto-merges stored IDs into
        // excludeCredentialIds on register and allowCredentialIds on
        // sign-in. Reference impl (BlockStoreCredentialRegistry) lives
        // in the passkey guide; copy-paste into your app.
        val registry = BlockStoreCredentialRegistry()
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "my-app.com",
            rpName = "My App",
            credentialRegistry = registry,
            onRegistryError = { op, err -> /* log */ },
        )
        val passkey = PasskeyClient(prfProvider, null, null)

        // Inspect / mutate via the credentials() sub-object.
        val known = passkey.credentials().get()

        // ANCHOR_END: with-credential-registry
    }
}
