# Credential metadata

Every passkey flow returns the credential it created or signed in with. Its IDs and attestation hints let you pin a returning user to the same wallet, prevent duplicate registrations, correlate with your backend, and show which authenticator holds the passkey.

`register`, `signIn`, and `connectWithPasskey` each return a `credential` field. The built-in platform providers always populate it; only PRF backends that don't surface one (CLI / file-backed / hardware) leave it unset.

## Fields

`PasskeyCredential` carries:

| Field | Available | Use it for |
|---|---|---|
| `credentialId` | always | [Pinning a returning user](#pin-a-returning-user-to-the-same-wallet) and [preventing duplicate registrations](#prevent-duplicate-registrations). |
| `userId` | registration | [Correlating with your backend](#correlate-the-credential-with-your-backend). |
| `aaguid` | registration | [Showing the authenticator](#show-the-authenticator-and-sync-status). Unverified. |
| `backupEligible` | registration | [Showing the sync status](#show-the-authenticator-and-sync-status). |

## Using the fields

Each of these is optional. The basic register and sign-in flows need none of them: reach for one only when your app wants that behavior.

### Pin a returning user to the same wallet

Each credential derives its own wallet seed, so a returning user must sign in with the same credential to re-open the same wallet.

Persist `credentialId` after registration and pass it as `allowCredentials` on `signIn`. The OS then offers only that credential. Omit `allowCredentials` and the OS picks any matching credential for your RP.

```kotlin
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
        // stored credentialId bytes
        allowCredentials = emptyList(),
    )
)
// Log.v("Breez", "${signInResponse.wallet.seed}") // Pass to connect() to open the wallet
// Log.v("Breez", "${signInResponse.wallet.label}") // Label this wallet was derived from
// Log.v("Breez", "${signInResponse.labels}")
// This passkey's labels (populated on discovery sign-in)
// Log.v("Breez", "${signInResponse.credential}") // Credential signed in with (credential_id only)
```



### Prevent duplicate registrations

Pass the user's already-registered credential IDs as `excludeCredentials` on `register`. When one is already on the device, the OS refuses to create a second and raises `PrfProviderError.CredentialAlreadyExists`: route that to `signIn` so the picker surfaces the existing credential.

```kotlin
return try {
    val response = passkey.register(
        RegisterRequest(
            label = "personal",
            // app-persisted credential IDs from prior registrations
            excludeCredentials = emptyList(),
        )
    )
    response.wallet
} catch (e: PrfProviderException.CredentialAlreadyExists) {
    // A matching credential already exists; sign in to it instead.
    val response = passkey.signIn(SignInRequest(label = "personal"))
    response.wallet
}
```



### Correlate the credential with your backend

If your backend ties passkeys to your own user accounts, `userId` is a stable identifier set at registration that links the two. The SDK surfaces it locally and never transmits it. Persist it with your user record, then match it on later sign-ins to tell which user is signing in.

This enables account-level controls the passkey layer can't enforce on its own:

- Cap how many passkeys (and wallets) one account may register.
- Revoke a lost credential server-side.
- List a user's registered devices in their settings.

### Show the authenticator and sync status

`aaguid` identifies the authenticator that created the passkey (Apple Passwords, Google Password Manager, a hardware key). Look it up in the community [AAGUID database](https://github.com/passkeydeveloper/passkey-authenticator-aaguids) for a name and icon. `backupEligible` tells you whether the passkey syncs across the user's devices.

> **Note:** `aaguid` and `backupEligible` are unverified and self-reported by the authenticator. Use them as display hints, never as a trust signal.

## Persisting the values

The use cases above require these values to be persisted across app launches. `credentialId` is returned on every authentication response, while `aaguid`, `backupEligible`, and `userId` are only returned during registration and should be stored at that time.

Use synced storage such as iCloud Keychain (iOS), Block Store (Android), or your own synced backend. Local-only storage is insufficient because it is lost on app reinstall and cannot be accessed from another device.
