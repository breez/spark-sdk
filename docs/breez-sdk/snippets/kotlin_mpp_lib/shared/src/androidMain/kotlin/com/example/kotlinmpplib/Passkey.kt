package com.example.kotlinmpplib

import android.app.Activity
import breez_sdk_spark.*
import technology.breez.spark.passkey.PasskeyClient
import technology.breez.spark.passkey.PasskeyProvider

// ANCHOR: implement-prf-provider
// Implement PrfProvider for a custom authenticator. Only deriveSeeds and
// isSupported are required.
class CustomPrfProvider : PrfProvider {
    override suspend fun deriveSeeds(request: DeriveSeedsRequest): DeriveSeedsOutput {
        // Return one 32-byte PRF output per salt, in input order.
        TODO("Implement using WebAuthn or native passkey APIs")
    }

    override suspend fun isSupported(): Boolean {
        TODO("Check platform passkey availability")
    }

    override suspend fun createPasskey(excludeCredentials: List<ByteArray>): PasskeyCredential {
        // Register a credential and return its ID plus attestation.
        TODO("Implement registration via native passkey API")
    }

    override suspend fun checkDomainAssociation(): DomainAssociation {
        return DomainAssociation.Skipped("CustomPrfProvider does not verify domain association")
    }
}
// ANCHOR_END: implement-prf-provider

class PasskeySnippets(private val activity: Activity) {
    suspend fun checkAvailability() {
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            options = PasskeyProviderOptions(rpId = "<your-rp-domain>", rpName = "Your App"),
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
        val passkey = PasskeyClient(
            breezApiKey = "<breez api key>",
            activityProvider = { activity },
            config = PasskeyConfig(providerOptions = PasskeyProviderOptions(rpId = "<your-rp-domain>", rpName = "Your App")),
        )
        // ANCHOR_END: setup-client
        return passkey
    }

    suspend fun connectWithPasskey(): BreezSdk {
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            options = PasskeyProviderOptions(rpId = "<your-rp-domain>", rpName = "Your App"),
        )
        val passkey = PasskeyClient(prfProvider, "<breez api key>", null)

        // ANCHOR: connect-with-passkey
        // Single-CTA onboarding: silent sign-in, fall through to register.
        val config = defaultConfig(Network.MAINNET).apply { apiKey = "<breez api key>" }
        val response = passkey.connectWithPasskey(
            ConnectWithPasskeyRequest(label = "personal")
        )

        val sdk = connect(ConnectRequest(config, response.wallet.seed, "./.data"))
        // ANCHOR_END: connect-with-passkey
        return sdk
    }

    suspend fun signInExistingUser(): SignInResponse {
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            options = PasskeyProviderOptions(rpId = "<your-rp-domain>", rpName = "Your App"),
        )
        val passkey = PasskeyClient(prfProvider, "<breez api key>", null)

        // ANCHOR: sign-in
        // Returning-user sign-in. No fall-through to register.
        return passkey.signIn(SignInRequest(label = "personal"))
        // ANCHOR_END: sign-in
    }

    suspend fun registerNewPasskey(): BreezSdk {
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            options = PasskeyProviderOptions(rpId = "<your-rp-domain>", rpName = "Your App"),
        )
        val passkey = PasskeyClient(prfProvider, "<breez api key>", null)

        // ANCHOR: register-passkey
        val config = defaultConfig(Network.MAINNET).apply { apiKey = "<breez api key>" }
        val response = passkey.register(RegisterRequest(label = "personal"))

        val sdk = connect(ConnectRequest(config, response.wallet.seed, "./.data"))
        // ANCHOR_END: register-passkey
        return sdk
    }

    suspend fun credentialMetadata() {
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            options = PasskeyProviderOptions(rpId = "<your-rp-domain>", rpName = "Your App"),
        )
        val passkey = PasskeyClient(prfProvider, "<breez api key>", null)

        // ANCHOR: credential-metadata
        val response = passkey.register(RegisterRequest(label = "personal"))

        response.credential?.let { credential ->
            // Log.v("Breez", "${credential.credentialId}") // Persist to reopen the same wallet on sign-in
            // Log.v("Breez", "${credential.aaguid}") // Authenticator model (display hint, unverified)
            // Log.v("Breez", "${credential.backupEligible}") // Whether the passkey syncs across devices
        }

        // Pin the stored credential ID so the OS can't substitute a sibling
        // credential, which would derive a different wallet.
        val signInResponse = passkey.signIn(
            SignInRequest(
                label = "personal",
                allowCredentials = emptyList(), // stored credentialId bytes
            )
        )
        // Log.v("Breez", "${signInResponse.wallet.seed}") // Pass to connect() to open the wallet
        // Log.v("Breez", "${signInResponse.wallet.label}") // Label this wallet was derived from
        // Log.v("Breez", "${signInResponse.labels}") // This passkey's labels (populated on discovery sign-in)
        // Log.v("Breez", "${signInResponse.credential}") // Credential signed in with (credential_id only)
        // ANCHOR_END: credential-metadata
    }

    suspend fun listLabels(): List<String> {
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            options = PasskeyProviderOptions(rpId = "<your-rp-domain>", rpName = "Your App"),
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
            options = PasskeyProviderOptions(rpId = "<your-rp-domain>", rpName = "Your App"),
        )
        val passkey = PasskeyClient(prfProvider, "<breez api key>", null)
        // ANCHOR: store-label
        passkey.labels().store("personal")
        // ANCHOR_END: store-label
    }

    suspend fun checkDomain() {
        // ANCHOR: domain-association
        // Diagnostic only: never blocks the ceremony.
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            options = PasskeyProviderOptions(rpId = "<your-rp-domain>", rpName = "Your App"),
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
            options = PasskeyProviderOptions(rpId = "<your-rp-domain>", rpName = "Your App"),
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
            // A matching credential already exists; sign in to it instead.
            val response = passkey.signIn(SignInRequest(label = "personal"))
            response.wallet
        }
        // ANCHOR_END: recover-already-exists
    }

    suspend fun handleTimeout(): SignInResponse {
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
            options = PasskeyProviderOptions(rpId = "<your-rp-domain>", rpName = "Your App"),
        )
        val passkey = PasskeyClient(prfProvider, "<breez api key>", null)

        // ANCHOR: handle-timeout
        return try {
            passkey.signIn(SignInRequest(label = "personal"))
        } catch (e: PrfProviderException.UserTimedOut) {
            // Show a retry UI. Do NOT auto-retry without user input.
            // Log.v("Breez", "Sign-in timed out: show \"Try Again\" UI.")
            throw e
        }
        // ANCHOR_END: handle-timeout
    }
}
