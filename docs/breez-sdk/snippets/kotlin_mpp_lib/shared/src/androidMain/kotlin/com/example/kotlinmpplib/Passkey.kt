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
    override suspend fun deriveSeeds(request: DeriveSeedsRequest): DeriveSeedsOutput {
        // Call platform passkey API with PRF extension. Use the dual-salt
        // ceremony when the authenticator supports it (one OS prompt for
        // N salts) and fall back to per-salt assertions otherwise.
        // Returns one 32-byte PRF output per salt in input order.
        TODO("Implement using WebAuthn or native passkey APIs")
    }

    override suspend fun isSupported(): Boolean {
        TODO("Check platform passkey availability")
    }

    override suspend fun createPasskey(excludeCredentials: List<ByteArray>): PasskeyCredential {
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
        // Pass `PasskeyProvider.BREEZ_RP_ID` instead of "<your-rp-domain>" if your
        // app is Breez-registered (shares credentials with other Breez apps).
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "<your-rp-domain>",
            rpName = "Your App",
        )
        val passkey = PasskeyClient(prfProvider, "<breez api key>", null)

        // ANCHOR: check-availability
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

    fun setupPasskeyClient(): PasskeyClient {
        // ANCHOR: setup-client
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "<your-rp-domain>",
            rpName = "Your App",
        )
        val passkey = PasskeyClient(prfProvider, "<breez api key>", null)
        // ANCHOR_END: setup-client
        return passkey
    }

    suspend fun connectWithPasskey(): BreezSdk {
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "<your-rp-domain>",
            rpName = "Your App",
        )
        val passkey = PasskeyClient(prfProvider, "<breez api key>", null)

        // ANCHOR: connect-with-passkey
        // Single-CTA onboarding: silent sign-in, fall through to register.
        val config = defaultConfig(Network.MAINNET).apply { apiKey = "<breez api key>" }
        val response = passkey.connectWithPasskey(
            ConnectWithPasskeyRequest(label = "personal")
        )

        // The credential is surfaced on both paths when the provider exposes it.
        response.credential?.let { credential ->
            val persistedId = credential.credentialId
        }

        val sdk = connect(ConnectRequest(config, response.wallet.seed, "./.data"))
        // ANCHOR_END: connect-with-passkey
        return sdk
    }

    suspend fun signInExistingUser(): SignInResponse {
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "<your-rp-domain>",
            rpName = "Your App",
        )
        val passkey = PasskeyClient(prfProvider, "<breez api key>", null)

        // ANCHOR: sign-in
        // Returning-user-only sign-in. No fall-through to register: use
        // `connectWithPasskey` when you also want the new-user path.
        return passkey.signIn(SignInRequest(label = "personal"))
        // ANCHOR_END: sign-in
    }

    suspend fun registerNewPasskey(): BreezSdk {
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "<your-rp-domain>",
            rpName = "Your App",
        )
        val passkey = PasskeyClient(prfProvider, "<breez api key>", null)

        // ANCHOR: register-passkey
        val config = defaultConfig(Network.MAINNET).apply { apiKey = "<breez api key>" }
        val response = passkey.register(RegisterRequest(label = "personal"))

        // Persist credentialId for future excludeCredentials.
        response.credential?.let { credential ->
            val persistedCredentialId = credential.credentialId
            val persistedUserId = credential.userId
        }

        val sdk = connect(ConnectRequest(config, response.wallet.seed, "./.data"))
        // ANCHOR_END: register-passkey
        return sdk
    }

    suspend fun credentialMetadata() {
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "<your-rp-domain>",
            rpName = "Your App",
        )
        val passkey = PasskeyClient(prfProvider, "<breez api key>", null)

        // ANCHOR: credential-metadata
        val response = passkey.register(RegisterRequest(label = "personal"))

        // Persist these in synced storage (Block Store / iCloud Keychain) so
        // they survive reinstall and reach the user's other devices. aaguid
        // and backupEligible are only available here, on registration.
        response.credential?.let { credential ->
            val persistedCredentialId = credential.credentialId
            val persistedAaguid = credential.aaguid
            val persistedBackupEligible = credential.backupEligible
        }

        // On a later sign-in, pin the stored credential ID via allowCredentials
        // so the OS cannot substitute a sibling credential, which would derive
        // a different wallet seed.
        passkey.signIn(
            SignInRequest(
                label = "personal",
                allowCredentials = emptyList(), // stored credentialId bytes
            )
        )
        // ANCHOR_END: credential-metadata
    }

    suspend fun listLabels(): List<String> {
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "<your-rp-domain>",
            rpName = "Your App",
        )
        val passkey = PasskeyClient(prfProvider, "<breez api key>", null)
        // ANCHOR: list-labels
        val labels = passkey.labels().list()
        for (label in labels) {
            // Log.v("Breez", "Found label: $label")
        }
        // ANCHOR_END: list-labels
        return labels
    }

    suspend fun storeLabel() {
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "<your-rp-domain>",
            rpName = "Your App",
        )
        val passkey = PasskeyClient(prfProvider, "<breez api key>", null)
        // ANCHOR: store-label
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
            rpId = "<your-rp-domain>",
            rpName = "Your App",
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
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "<your-rp-domain>",
            rpName = "Your App",
        )
        val passkey = PasskeyClient(prfProvider, "<breez api key>", null)

        // ANCHOR: recover-already-exists
        return try {
            val response = passkey.register(
                RegisterRequest(
                    label = "personal",
                    excludeCredentials = emptyList(), // app-persisted credential IDs from prior registrations
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
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            rpId = "<your-rp-domain>",
            rpName = "Your App",
        )
        val passkey = PasskeyClient(prfProvider, "<breez api key>", null)

        // ANCHOR: handle-timeout
        return try {
            passkey.signIn(SignInRequest(label = "personal"))
        } catch (e: PrfProviderException.UserTimedOut) {
            // Log.v("Breez", "Sign-in timed out: show \"Try Again\" UI.")
            throw e
        }
        // ANCHOR_END: handle-timeout
    }
}
