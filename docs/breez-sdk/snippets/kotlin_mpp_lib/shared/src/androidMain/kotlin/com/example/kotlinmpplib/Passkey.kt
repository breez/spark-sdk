package com.example.kotlinmpplib

import android.app.Activity
import breez_sdk_spark.*
import technology.breez.spark.passkey.PasskeyProvider

// ANCHOR: implement-prf-provider
// Implement the PrfProvider interface for custom logic if the built-in
// PasskeyProvider doesn't fit your needs (hardware key, FIDO2 transport,
// air-gapped backup file, etc.). Single API surface: deriveSeeds for
// derivation, createPasskey for registration, isSupported /
// checkDomainAssociation for diagnostics. Single-salt derivation is the
// trivial 1-element bulk case.
class CustomPrfProvider : PrfProvider {
    override suspend fun deriveSeeds(salts: List<String>): List<ByteArray> {
        // Call platform passkey API with PRF extension. Use the dual-salt
        // ceremony when the authenticator supports it (one OS prompt for
        // N salts) and fall back to per-salt assertions otherwise.
        // Returns one 32-byte PRF output per salt in input order.
        TODO("Implement using WebAuthn or native passkey APIs")
    }

    override suspend fun isSupported(): Boolean {
        // Check if a PRF-capable authenticator is reachable from this
        // platform / device.
        TODO("Check platform passkey availability")
    }

    override suspend fun createPasskey(request: CreatePasskeyRequest): RegisteredCredential {
        // Register a new credential and return its ID + AAGUID + BE flag.
        TODO("Implement registration via native passkey API")
    }

    override suspend fun checkDomainAssociation(): DomainAssociation {
        // Optional: verify the app's identity against the platform's
        // domain verification source (e.g., Android Digital Asset Links
        // for the built-in PasskeyProvider). Custom providers without a
        // verification source return Skipped, which tells callers
        // "proceed with WebAuthn as normal".
        return DomainAssociation.Skipped("CustomPrfProvider does not verify domain association")
    }
}
// ANCHOR_END: implement-prf-provider

class PasskeySnippets(private val activity: Activity) {
    suspend fun checkAvailability() {
        // ANCHOR: check-availability
        val prfProvider = PasskeyProvider(
            activityProvider = { activity }, // provide the current Activity
        )
        if (prfProvider.isSupported()) {
            // Show passkey as primary option
        } else {
            // Fall back to mnemonic flow
        }
        // ANCHOR_END: check-availability
    }

    suspend fun connectWithPasskey(): BreezSdk {
        // ANCHOR: connect-with-passkey
        val prfProvider = PasskeyProvider(
            activityProvider = { activity }, // provide the current Activity
        )
        val passkey = PasskeyClient(prfProvider, null)

        // signIn derives the wallet seed for an existing credential. With
        // bulk PRF on iOS+Android this is a single OS prompt that derives
        // master + label seeds in one ceremony.
        val response = passkey.signIn(SignInRequest(label = "personal", extraSalts = emptyList()))

        val config = defaultConfig(Network.MAINNET)
        val sdk = connect(ConnectRequest(config, response.wallet.seed, "./.data"))
        // ANCHOR_END: connect-with-passkey
        return sdk
    }

    suspend fun registerNewPasskey(): BreezSdk {
        // ANCHOR: register-passkey
        // For a brand-new user with no existing passkey: register() creates
        // the credential AND derives the wallet seed in one orchestrated
        // call. On iOS+Android this is 2 OS prompts total (1 create + 1
        // dual-salt assert) thanks to the SDK's bulk-PRF setup_wallet path.
        val prfProvider = PasskeyProvider(
            activityProvider = { activity }, // provide the current Activity
        )
        val passkey = PasskeyClient(prfProvider, null)

        val response = passkey.register(
            RegisterRequest(
                label = "personal",
                extraSalts = emptyList(),
                excludeCredentialIds = emptyList(),
            )
        )

        val config = defaultConfig(Network.MAINNET)
        val sdk = connect(ConnectRequest(config, response.wallet.seed, "./.data"))
        // ANCHOR_END: register-passkey
        return sdk
    }

    suspend fun listLabels(): List<String> {
        // ANCHOR: list-labels
        val prfProvider = PasskeyProvider(
            activityProvider = { activity }, // provide the current Activity
        )
        val relayConfig = NostrRelayConfig(breezApiKey = "<breez api key>")
        val passkey = PasskeyClient(prfProvider, relayConfig)

        // signIn with no label runs in discovery mode: it derives the
        // master seed AND lists labels in the same ceremony, so a follow-up
        // listLabels() reads from the cached identity for free.
        val labels = passkey.listLabels()

        for (label in labels) {
            // Log.v("Breez", "Found label: $label")
        }
        // ANCHOR_END: list-labels
        return labels
    }

    suspend fun storeLabel() {
        // ANCHOR: store-label
        val prfProvider = PasskeyProvider(
            activityProvider = { activity }, // provide the current Activity
        )
        val relayConfig = NostrRelayConfig(breezApiKey = "<breez api key>")
        val passkey = PasskeyClient(prfProvider, relayConfig)

        // For a new label on an existing identity, call signIn(newLabel)
        // first to seed the SDK's identity cache via setup_wallet, THEN
        // storeLabel uses the cached identity for free (1 OS prompt total).
        passkey.storeLabel("personal")
        // ANCHOR_END: store-label
    }

    suspend fun singleCtaOnboarding(): Wallet {
        // ANCHOR: signin-fallback-register
        // Single-CTA onboarding: try silent signIn first, fall through to
        // register on CredentialNotFound. The OS shows ONE prompt for a
        // returning user (silent assertion succeeds), TWO for a new user
        // (silent assertion fast-fails, then create + dual-salt assert).
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
        )
        val passkey = PasskeyClient(prfProvider, null)

        return try {
            // Discovery mode (label=null): derives master + DEFAULT label
            // in a single ceremony. The fresh-device user fast-fails in
            // <300ms with no UI shown.
            val response = passkey.signIn(SignInRequest(label = null, extraSalts = emptyList()))
            response.wallet
        } catch (e: PrfProviderException.CredentialNotFound) {
            // CredentialNotFound is the SDK's classification for "no
            // matching credential on this device", including iOS's
            // <300ms fast-fail case where the platform conflates no-cred
            // with user-cancel.
            val response = passkey.register(
                RegisterRequest(
                    label = "personal",
                    extraSalts = emptyList(),
                    excludeCredentialIds = emptyList(),
                )
            )
            response.wallet
        }
        // ANCHOR_END: signin-fallback-register
    }

    suspend fun checkDomain() {
        // ANCHOR: domain-association
        // Verify Apple AASA / Android Asset Links / Web Related Origins
        // before the first WebAuthn ceremony. Diagnostic only: never blocks.
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
        )
        val result = prfProvider.checkDomainAssociation()

        when (result) {
            is DomainAssociation.Associated -> {
                // Safe to proceed.
            }
            is DomainAssociation.NotAssociated -> {
                // Configuration is wrong (entitlement missing, AASA stale,
                // assetlinks malformed). Surface a developer-facing error.
                // Log.e("Breez", "Domain association failed (source=${result.source}): ${result.reason}")
            }
            is DomainAssociation.Skipped -> {
                // Verification could not be performed (offline, endpoint
                // timeout, no public-suffix match). Proceed normally:
                // this is NOT a negative signal.
            }
        }
        // ANCHOR_END: domain-association
    }

    suspend fun recoverFromAlreadyExists(): Wallet {
        // ANCHOR: recover-already-exists
        // The OS rejected register because the user's password manager
        // already holds a credential matching `excludeCredentialIds`.
        // Route the user to the sign-in path: the OS picker will surface
        // the existing credential and the SDK's identity cache will warm
        // up on the assertion.
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
        )
        val passkey = PasskeyClient(prfProvider, null)

        return try {
            val response = passkey.register(
                RegisterRequest(
                    label = "personal",
                    extraSalts = emptyList(),
                    excludeCredentialIds = emptyList(), // app-persisted credential IDs from prior registrations
                )
            )
            response.wallet
        } catch (e: PrfProviderException.CredentialAlreadyExists) {
            // Flip to sign-in. The existing credential's PRF output is
            // the same wallet seed the host would have minted on register.
            val response = passkey.signIn(
                SignInRequest(label = "personal", extraSalts = emptyList())
            )
            response.wallet
        }
        // ANCHOR_END: recover-already-exists
    }

    suspend fun handleTimeout(): SignInResponse {
        // ANCHOR: handle-timeout
        // The OS biometric inactivity timeout (~55s+) tore down the prompt
        // without user intent. Distinct from a real cancel: hosts may
        // surface a re-prompt UI without treating it as the user opting
        // out. The SDK fires PrfProviderException.UserTimedOut when
        // assertion or register elapsed time crosses 55_000 ms.
        val prfProvider = PasskeyProvider(
            activityProvider = { activity },
        )
        val passkey = PasskeyClient(prfProvider, null)

        return try {
            passkey.signIn(SignInRequest(label = "personal", extraSalts = emptyList()))
        } catch (e: PrfProviderException.UserTimedOut) {
            // Show a sticky retry screen with timeout-specific copy.
            // Do NOT auto-retry without user input.
            // Log.v("Breez", "Sign-in timed out: show \"Try Again\" UI.")
            throw e
        }
        // ANCHOR_END: handle-timeout
    }
}
