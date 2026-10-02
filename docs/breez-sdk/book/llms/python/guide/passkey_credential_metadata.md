# Credential metadata

Every passkey flow returns the credential it created or signed in with. Its IDs and attestation hints let you pin a returning user to the same wallet, prevent duplicate registrations, correlate with your backend, and show which authenticator holds the passkey.

`register`, `sign_in`, and `connect_with_passkey` each return a `credential` field. The built-in platform providers always populate it; only PRF backends that don't surface one (CLI / file-backed / hardware) leave it unset.

## Fields

`PasskeyCredential` carries:

| Field | Available | Use it for |
|---|---|---|
| `credential_id` | always | [Pinning a returning user](#pin-a-returning-user-to-the-same-wallet) and [preventing duplicate registrations](#prevent-duplicate-registrations). |
| `user_id` | registration | [Correlating with your backend](#correlate-the-credential-with-your-backend). |
| `aaguid` | registration | [Showing the authenticator](#show-the-authenticator-and-sync-status). Unverified. |
| `backup_eligible` | registration | [Showing the sync status](#show-the-authenticator-and-sync-status). |

## Using the fields

Each of these is optional. The basic register and sign-in flows need none of them: reach for one only when your app wants that behavior.

### Pin a returning user to the same wallet

Each credential derives its own wallet seed, so a returning user must sign in with the same credential to re-open the same wallet.

Persist `credential_id` after registration and pass it as `allow_credentials` on `sign_in`. The OS then offers only that credential. Omit `allow_credentials` and the OS picks any matching credential for your RP.

```python
response = await passkey.register(RegisterRequest(label="personal"))

if response.credential is not None:
    # Persist to reopen the same wallet on sign-in
    print(response.credential.credential_id)
    # Authenticator model (display hint, unverified)
    print(response.credential.aaguid)
    # Whether the passkey syncs across devices
    print(response.credential.backup_eligible)

# Pin the stored credential ID so the OS can't substitute a sibling
# credential, which would derive a different wallet.
sign_in_response = await passkey.sign_in(
    SignInRequest(
        label="personal",
        allow_credentials=[
            # stored credential_id bytes
        ],
    )
)
# Pass to connect() to open the wallet
print(sign_in_response.wallet.seed)
# Label this wallet was derived from
print(sign_in_response.wallet.label)
# This passkey's labels (populated on discovery sign-in)
print(sign_in_response.labels)
# Credential signed in with (credential_id only)
print(sign_in_response.credential)
```



### Prevent duplicate registrations

Pass the user's already-registered credential IDs as `exclude_credentials` on `register`. When one is already on the device, the OS refuses to create a second and raises `PrfProviderError.CREDENTIAL_ALREADY_EXISTS`: route that to `sign_in` so the picker surfaces the existing credential.

```python
try:
    await passkey.register(
        RegisterRequest(
            label="personal",
            exclude_credentials=[
                # app-persisted credential IDs from prior registrations
            ],
        )
    )
except PrfProviderError.CredentialAlreadyExists:
    # A matching credential already exists; sign in to it instead.
    response = await passkey.sign_in(SignInRequest(label="personal"))
    return response.wallet
```



### Correlate the credential with your backend

If your backend ties passkeys to your own user accounts, `user_id` is a stable identifier set at registration that links the two. The SDK surfaces it locally and never transmits it. Persist it with your user record, then match it on later sign-ins to tell which user is signing in.

This enables account-level controls the passkey layer can't enforce on its own:

- Cap how many passkeys (and wallets) one account may register.
- Revoke a lost credential server-side.
- List a user's registered devices in their settings.

### Show the authenticator and sync status

`aaguid` identifies the authenticator that created the passkey (Apple Passwords, Google Password Manager, a hardware key). Look it up in the community [AAGUID database](https://github.com/passkeydeveloper/passkey-authenticator-aaguids) for a name and icon. `backup_eligible` tells you whether the passkey syncs across the user's devices.

> **Note:** `aaguid` and `backup_eligible` are unverified and self-reported by the authenticator. Use them as display hints, never as a trust signal.

## Persisting the values

The use cases above require these values to be persisted across app launches. `credential_id` is returned on every authentication response, while `aaguid`, `backup_eligible`, and `user_id` are only returned during registration and should be stored at that time.

Use synced storage such as iCloud Keychain (iOS), Block Store (Android), or your own synced backend. Local-only storage is insufficient because it is lost on app reinstall and cannot be accessed from another device.
