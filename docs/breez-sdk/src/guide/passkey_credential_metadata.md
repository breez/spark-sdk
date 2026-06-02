# Credential metadata

Every passkey flow returns the credential it used or created. Its IDs and attestation hints let you keep a returning user on the same wallet, prevent duplicate registrations on one device, correlate the credential with your backend, and show which authenticator holds the passkey.

{{#name register}}, {{#name sign_in}}, and {{#name connect_with_passkey}} each return a {{#name credential}} field. It is unset only for PRF backends that do not surface one (CLI / file-backed / hardware providers); the built-in platform providers always populate it.

## Fields

{{#name PasskeyCredential}} carries:

| Field | Available | Use it for |
|---|---|---|
| {{#name credential_id}} | always | [Pinning a returning user](#pin-a-returning-user-to-the-same-wallet) and [preventing duplicate registrations](#prevent-duplicate-registrations). |
| {{#name user_id}} | registration | [Correlating with your backend](#correlate-the-credential-with-your-backend). |
| {{#name aaguid}} | registration | [Showing the authenticator](#show-the-authenticator-and-sync-status). Unverified. |
| {{#name backup_eligible}} | registration | [Showing the sync status](#show-the-authenticator-and-sync-status). |

The attestation fields ({{#name user_id}}, {{#name aaguid}}, {{#name backup_eligible}}) come from the registration ceremony only. A sign-in assertion carries no attestation, so they are unset on the sign-in path; {{#name credential_id}} is always present.

## Using the fields

Each of these is optional. The basic register and sign-in flows need none of them: reach for one only when your app wants that specific behavior.

### Pin a returning user to the same wallet

Each credential derives its own wallet seed, so a returning user must sign in with the same credential to re-open the same wallet. Persist {{#name credential_id}} after registration and pass it as {{#name allow_credentials}} on {{#name sign_in}}: the OS then offers only that credential and the user lands on the same wallet. {{#name allow_credentials}} is optional; omit it and the OS picks any matching credential for your RP.

{{#tabs passkey:credential-metadata}}

### Prevent duplicate registrations

Pass the credential IDs already registered for this user as {{#name exclude_credentials}} on {{#name register}}. When one matches a credential already on the device, the OS refuses to create a second and raises {{#enum PrfProviderError::CredentialAlreadyExists}}; route that to {{#name sign_in}} so the OS picker surfaces the existing credential. {{#name exclude_credentials}} is optional; omit it for the simple path.

{{#tabs passkey:recover-already-exists}}

### Correlate the credential with your backend

Most wallets don't need this, but if your backend ties wallets to your own user accounts, {{#name user_id}} is the stable identifier that links the two. The SDK mints this WebAuthn user handle at registration and surfaces it here (it never transmits the value). Persist it with your user record, then match it against the handle a later assertion carries to tell which of your users is signing in.

Tying credentials to accounts this way unlocks controls the passkey layer can't enforce on its own: capping how many passkeys (and therefore wallets) one account may register, revoking a lost credential server-side, or listing a user's registered devices in their account settings.

### Show the authenticator and sync status

{{#name aaguid}} identifies the authenticator that created the passkey (Apple Passwords, Google Password Manager, a hardware key, and so on). Look it up in the community [AAGUID database](https://github.com/passkeydeveloper/passkey-authenticator-aaguids) for a display name and icon. {{#name backup_eligible}} tells you whether the passkey syncs across the user's devices or stays on this one.

Both are unverified attestation, self-reported by the authenticator: use them as display hints only, never as a trust or security signal.

## Persisting the values

The use cases above need the values kept across app launches. {{#name credential_id}} comes back on every response; {{#name aaguid}}, {{#name backup_eligible}}, and {{#name user_id}} arrive only at registration, so capture them there. For cross-device continuity, back the store with synced storage: iCloud Keychain (`kSecAttrSynchronizable`) on iOS, Block Store on Android, or your own synced backend. Plain local storage does not survive reinstall or replicate to a second device.
