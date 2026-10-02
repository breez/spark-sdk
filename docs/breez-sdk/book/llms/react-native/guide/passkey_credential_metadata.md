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

```typescript
const response = await passkey.register({ label: 'personal', excludeCredentials: undefined })

if (response.credential !== undefined) {
  // Persist to reopen the same wallet on sign-in
  console.log(response.credential.credentialId)
  // Authenticator model (display hint, unverified)
  console.log(response.credential.aaguid)
  // Whether the passkey syncs across devices
  console.log(response.credential.backupEligible)
}

// Pin the stored credential ID so the OS can't substitute a sibling
// credential, which would derive a different wallet.
const signInResponse = await passkey.signIn({
  label: 'personal',
  allowCredentials: [/* stored credentialId bytes */],
  preferImmediatelyAvailableCredentials: undefined
})
// Pass to connect() to open the wallet
console.log(signInResponse.wallet.seed)
// Label this wallet was derived from
console.log(signInResponse.wallet.label)
// This passkey's labels (populated on discovery sign-in)
console.log(signInResponse.labels)
// Credential signed in with (credential_id only)
console.log(signInResponse.credential)
```



### Prevent duplicate registrations

Pass the user's already-registered credential IDs as `excludeCredentials` on `register`. When one is already on the device, the OS refuses to create a second and raises `PrfProviderError.CredentialAlreadyExists`: route that to `signIn` so the picker surfaces the existing credential.

```typescript
try {
  const response = await passkey.register({
    label: 'personal',
    excludeCredentials: [
      // app-persisted credential IDs from prior registrations
    ]
  })
  return response.wallet
} catch (error) {
  if (error instanceof PasskeyPrfException && error.code === 'credentialAlreadyExists') {
    // A matching credential already exists; sign in to it instead.
    const response = await passkey.signIn({
      label: 'personal',
      allowCredentials: undefined,
      preferImmediatelyAvailableCredentials: undefined
    })
    return response.wallet
  }
  throw error
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
