# Credential metadata

Every passkey flow returns the credential it used or created, so your app can manage passkeys itself: pin a specific credential on sign-in, prevent duplicate registrations, and build a settings screen that lists registered passkeys by provider and backup status.

{{#name register}}, {{#name sign_in}}, and {{#name connect_with_passkey}} each return a `credential` field. It is unset only for PRF backends that do not surface one (CLI / file-backed / hardware providers); the built-in platform providers always populate it.

## Fields

`PasskeyCredential` carries:

| Field | Available | Use it for |
|---|---|---|
| {{#name credential_id}} | always | Pinning later ceremonies via `allow_credentials` / `exclude_credentials`. |
| {{#name user_id}} | registration | The WebAuthn user handle the provider minted. |
| {{#name aaguid}} | registration | Showing the authenticator / provider (see below). Unverified. |
| {{#name backup_eligible}} | registration | Showing whether the passkey syncs across the user's devices. |

The attestation fields ({{#name user_id}}, {{#name aaguid}}, {{#name backup_eligible}}) come from the registration ceremony only. A sign-in assertion carries no attestation, so they are unset on the sign-in path; {{#name credential_id}} is always present.

## Recording credentials

Persist {{#name credential_id}} from every response in your own storage. On registration, also persist {{#name aaguid}} and {{#name backup_eligible}}: they are not available again on later sign-ins.

{{#tabs passkey:credential-metadata}}

For cross-device continuity, back this store with synced storage: iCloud Keychain (`kSecAttrSynchronizable`) on iOS, Block Store on Android, or your own synced backend. Plain local storage does not survive reinstall or replicate to a second device.

## Pinning a credential on sign-in

The sign-in call in the snippet above passes a stored {{#name credential_id}} as `allow_credentials` to constrain the assertion to one credential. This matters for security-sensitive flows. With an empty `allow_credentials`, the OS picker may resolve a different (sibling) credential for the same Relying Party, and a different credential derives a different wallet seed. Pin `allow_credentials` to the active credential before revealing anything tied to the active wallet, such as a recovery phrase.

## Preventing duplicate registration

Pass your stored credential IDs as `exclude_credentials` on {{#name register}} so the OS refuses to create a second credential on a device that already has one. It raises {{#enum PrfProviderError::CredentialAlreadyExists}}, which you route to the sign-in path:

{{#tabs passkey:recover-already-exists}}

## Showing the authenticator provider (AAGUID)

The {{#name aaguid}} identifies the authenticator model (iCloud Keychain, Google Password Manager, a password manager, a hardware key, and so on). Map it to a display name and icon with the community AAGUID database at <https://github.com/passkeydeveloper/passkey-authenticator-aaguids>, and render it in a passkey-management screen next to {{#name backup_eligible}} ("syncs across your devices" vs "this device only").

AAGUID is self-reported by the authenticator and unverified. Use it for display only, never as a trust or security signal.
