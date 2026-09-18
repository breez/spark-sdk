# Credential metadata

Every passkey flow returns the credential it created or signed in with. Its IDs and attestation hints let you pin a returning user to the same wallet, prevent duplicate registrations, correlate with your backend, and show which authenticator holds the passkey.

`Register`, `SignIn`, and `ConnectWithPasskey` each return a `Credential` field. The built-in platform providers always populate it; only PRF backends that don't surface one (CLI / file-backed / hardware) leave it unset.

## Fields

`PasskeyCredential` carries:

| Field | Available | Use it for |
|---|---|---|
| `CredentialId` | always | [Pinning a returning user](#pin-a-returning-user-to-the-same-wallet) and [preventing duplicate registrations](#prevent-duplicate-registrations). |
| `UserId` | registration | [Correlating with your backend](#correlate-the-credential-with-your-backend). |
| `Aaguid` | registration | [Showing the authenticator](#show-the-authenticator-and-sync-status). Unverified. |
| `BackupEligible` | registration | [Showing the sync status](#show-the-authenticator-and-sync-status). |

## Using the fields

Each of these is optional. The basic register and sign-in flows need none of them: reach for one only when your app wants that behavior.

### Pin a returning user to the same wallet

Each credential derives its own wallet seed, so a returning user must sign in with the same credential to re-open the same wallet.

Persist `CredentialId` after registration and pass it as `AllowCredentials` on `SignIn`. The OS then offers only that credential. Omit `AllowCredentials` and the OS picks any matching credential for your RP.

```go
label := "personal"
response, err := passkey.Register(breez_sdk_spark.RegisterRequest{Label: &label})
if err != nil {
	return err
}

if response.Credential != nil {
	// Persist to reopen the same wallet on sign-in
	log.Println(response.Credential.CredentialId)
	// Authenticator model (display hint, unverified)
	log.Println(response.Credential.Aaguid)
	// Whether the passkey syncs across devices
	log.Println(response.Credential.BackupEligible)
}

// Pin the stored credential ID so the OS can't substitute a sibling
// credential, which would derive a different wallet.
signInResponse, err := passkey.SignIn(breez_sdk_spark.SignInRequest{
	Label:            &label,
	AllowCredentials: &[][]byte{
		// stored CredentialId bytes
	},
})
if err != nil {
	return err
}
// Pass to connect() to open the wallet
log.Println(signInResponse.Wallet.Seed)
// Label this wallet was derived from
log.Println(signInResponse.Wallet.Label)
// This passkey's labels (populated on discovery sign-in)
log.Println(signInResponse.Labels)
// Credential signed in with (credential_id only)
log.Println(signInResponse.Credential)
```



### Prevent duplicate registrations

Pass the user's already-registered credential IDs as `ExcludeCredentials` on `Register`. When one is already on the device, the OS refuses to create a second and raises `PrfProviderErrorCredentialAlreadyExists`: route that to `SignIn` so the picker surfaces the existing credential.

```go
label := "personal"
registerResponse, err := passkey.Register(breez_sdk_spark.RegisterRequest{
	Label:              &label,
	ExcludeCredentials: &[][]byte{
		// app-persisted credential IDs from prior registrations
	},
})
if err == nil {
	return &registerResponse.Wallet, nil
}

if !errors.Is(err, breez_sdk_spark.ErrPrfProviderErrorCredentialAlreadyExists) {
	return nil, err
}

// A matching credential already exists; sign in to it instead.
signInResponse, err := passkey.SignIn(breez_sdk_spark.SignInRequest{Label: &label})
if err != nil {
	return nil, err
}
return &signInResponse.Wallet, nil
```



### Correlate the credential with your backend

If your backend ties passkeys to your own user accounts, `UserId` is a stable identifier set at registration that links the two. The SDK surfaces it locally and never transmits it. Persist it with your user record, then match it on later sign-ins to tell which user is signing in.

This enables account-level controls the passkey layer can't enforce on its own:

- Cap how many passkeys (and wallets) one account may register.
- Revoke a lost credential server-side.
- List a user's registered devices in their settings.

### Show the authenticator and sync status

`Aaguid` identifies the authenticator that created the passkey (Apple Passwords, Google Password Manager, a hardware key). Look it up in the community [AAGUID database](https://github.com/passkeydeveloper/passkey-authenticator-aaguids) for a name and icon. `BackupEligible` tells you whether the passkey syncs across the user's devices.

> **Note:** `Aaguid` and `BackupEligible` are unverified and self-reported by the authenticator. Use them as display hints, never as a trust signal.

## Persisting the values

The use cases above require these values to be persisted across app launches. `CredentialId` is returned on every authentication response, while `Aaguid`, `BackupEligible`, and `UserId` are only returned during registration and should be stored at that time.

Use synced storage such as iCloud Keychain (iOS), Block Store (Android), or your own synced backend. Local-only storage is insufficient because it is lost on app reinstall and cannot be accessed from another device.
