# Credential metadata

Every passkey flow returns the credential it created or signed in with. Its IDs and attestation hints let you pin a returning user to the same wallet, prevent duplicate registrations, correlate with your backend, and show which authenticator holds the passkey.

{{#name register}}, {{#name sign_in}}, and {{#name connect_with_passkey}} each return a {{#name credential}} field. The built-in platform providers always populate it; only PRF backends that don't surface one (CLI / file-backed / hardware) leave it unset.

## Fields

{{#name PasskeyCredential}} carries:

| Field | Available | Use it for |
|---|---|---|
| {{#name credential_id}} | always | [Pinning a returning user](#pin-a-returning-user-to-the-same-wallet) and [preventing duplicate registrations](#prevent-duplicate-registrations). |
| {{#name user_id}} | registration | [Correlating with your backend](#correlate-the-credential-with-your-backend). |
| {{#name aaguid}} | registration | [Showing the authenticator](#show-the-authenticator-and-sync-status). Unverified. |
| {{#name backup_eligible}} | registration | [Showing the sync status](#show-the-authenticator-and-sync-status). |

## Using the fields

Each of these is optional. The basic register and sign-in flows need none of them: reach for one only when your app wants that behavior.

### Pin a returning user to the same wallet

Each credential derives its own wallet seed, so a returning user must sign in with the same credential to re-open the same wallet.

Persist {{#name credential_id}} after registration and pass it as {{#name allow_credentials}} on {{#name sign_in}}. The OS then offers only that credential. Omit {{#name allow_credentials}} and the OS picks any matching credential for your RP.

{{#tabs passkey:credential-metadata}}

### Prevent duplicate registrations

Pass the user's already-registered credential IDs as {{#name exclude_credentials}} on {{#name register}}. When one is already on the device, the OS refuses to create a second and raises {{#enum PrfProviderError::CredentialAlreadyExists}}: route that to {{#name sign_in}} so the picker surfaces the existing credential.

{{#tabs passkey:recover-already-exists}}

### Correlate the credential with your backend

If your backend ties passkeys to your own user accounts, {{#name user_id}} is a stable identifier set at registration that links the two. The SDK surfaces it locally and never transmits it. Persist it with your user record, then match it on later sign-ins to tell which user is signing in.

This enables account-level controls the passkey layer can't enforce on its own:

- Cap how many passkeys (and wallet) one account may register.
- Revoke a lost credential server-side.
- List a user's registered devices in their settings.

### Show the authenticator and sync status

{{#name aaguid}} identifies the authenticator that created the passkey (Apple Passwords, Google Password Manager, a hardware key). Look it up in the community [AAGUID database](https://github.com/passkeydeveloper/passkey-authenticator-aaguids) for a name and icon. {{#name backup_eligible}} tells you whether the passkey syncs across the user's devices.

> **Note:** {{#name aaguid}} and {{#name backup_eligible}} are unverified and self-reported by the authenticator. Use them as display hints, never as a trust signal.

## Persisting the values

The use cases above require these values to be persisted across app launches. {{#name credential_id}} is returned on every authentication response, while {{#name aaguid}},{{#name backup_eligible}}, and {{#name user_id}} are only returned during registration and should be stored at that time.

Use synced storage such as iCloud Keychain (iOS), Block Store (Android), or your own synced backend. Local-only storage is insufficient because it is lost on app reinstall and cannot be accessed from another device.
