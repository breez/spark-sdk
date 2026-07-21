## Login & backup

How users get into a wallet and how they recover it are one story: the login method determines the backup story. These guidelines cover both.

### UX principles

- **Onboarding should be seamless.** Don't block first use with backup ceremonies.
- **Passkey-first where supported.** Passkeys remove the mnemonic from onboarding entirely while preserving self-custody. Be transparent about the trust model: the passkey is the wallet, not a convenience layer on top of it.
- **The mnemonic is the universal fallback.** It remains the onboarding path on devices without passkey support, and the manual backup path for everyone else.
- **OS prompts are precious.** Every biometric prompt costs trust and attention. Design flows around the minimum number of prompts, and treat a dismissed prompt as a user decision, not an error to retry.

### Onboarding

1. **Only offer what the device can deliver.** Check passkey availability at startup ({{#name PasskeyClient.check_availability}}) and fall back to mnemonic onboarding when the device or configuration can't support passkeys. One check covers both.
2. **One button when the platform allows it.** On iOS and Android, a single primary action ({{#name PasskeyClient.connect_with_passkey}}) serves both new and returning users: it signs a returning user in silently and falls through to registration for a new one. On web, do the same when the browser can probe for credentials silently; otherwise split into "Create a new passkey" and "Sign in with a passkey", because WebAuthn can't distinguish "no credential" from "user cancelled".
3. **Keep the mnemonic path reachable but quiet**: a small "Use recovery phrase instead" link under the primary action, not a competing button.
4. **Say what the passkey is before creating it.** A short warning that the passkey is how the user accesses their funds, and that deleting it may make funds permanently inaccessible. Beyond that, add no consent screen of your own: the OS already shows one.
5. **Narrate the wait.** Passkey setup has several distinct phases; label each one specifically ("Verifying app domain...", "Detecting passkey...", "Initializing...") rather than showing one generic spinner.
6. **For mnemonic onboarding**, present the generated phrase as a numbered word grid with copy support, and require an explicit "I've saved my phrase" confirmation before moving on.

### Returning users

1. **Day-2 login is effortless.** Don't run a full passkey ceremony on every launch: on native, store the seed in the keychain so the wallet opens directly (optionally biometric-bound, making the return a single biometric prompt); on web, offer "Sign in with a passkey". Always leave an escape hatch ("Use a different wallet").
2. **The mnemonic never touches plain storage.** On native, keep it in the keychain or secure storage; on web, re-derive it from the passkey each session. Within a session, cache the derived seed in memory so later SDK calls don't re-prompt.
3. **A dismissed prompt is an answer.** Never auto-retry: land on a persistent error state with a "Try Again" button, and re-prompt only on an explicit tap. (The SDK follows the same rule and never re-fires the OS prompt on its own.)
4. **Catch duplicate registrations.** When the authenticator reports that a passkey already exists for this wallet, say so and pivot the user to sign-in instead of silently prompting again.

### Backup

1. **Backup follows value, not signup.** Invite the user to back up after the wallet is created, or after the first payment arrives, never as a gate in front of first use.
2. **Explain, then verify.** Say plainly why the phrase must be written down and stored safely, and consider confirming the backup with a partial re-entry.
3. **Passkey users still get a recovery phrase.** Offer a user-initiated "Show recovery phrase" that derives the mnemonic on demand ({{#name PasskeyClient.sign_in}}), so losing the passkey is survivable. Require the re-authentication to use the same credential that owns the wallet, so a sibling credential can't reveal a different wallet's phrase.
4. **Protect phrase screens from capture.** Enable screen-capture protection wherever a mnemonic is displayed or entered.

### Platform notes

Browsers and native authenticators expose different error semantics, so the recommended flows differ by platform.

**iOS 18+ / Android 9+:** one "Use Passkey" button. A returning user gets a single biometric prompt; a new user fast-fails silently and falls through to registration. On a real cancel, show the persistent retry state and do not auto-register.

| Path | OS prompts |
|---|---|
| Returning user | **1** (one assertion derives master + label) |
| New user | **2** (1 create, 1 assertion) |

**Web:** "Create a new passkey" and "Sign in with a passkey" as separate actions when silent probing isn't available. {{#name PasskeyClient.connect_with_passkey}} is not surfaced on the WASM target.

**Multiple wallets per passkey:** when a user adds a wallet under a new label, sign in with the new label first and store the label second; that order costs one OS prompt instead of two. See [Managing labels](./passkey_labels.md).

### Continuity & recovery

- **Keep returning users on the same wallet.** Persist the credential metadata returned by each flow so the app recognizes the user's passkey, prevents duplicate registrations on the same device, and can show which authenticator holds the passkey and whether it syncs. Treat authenticator identity and backup flags as display hints, never as trust signals. See [Credential metadata](./passkey_credential_metadata.md).
- **Every failure has a next step.** Passkey errors normalize to {{#name PrfProviderError}} variants, each mapping to a recovery action; see the [error-recovery table](./passkey_onboarding.md#error-recovery). On iOS, the SDK disambiguates the platform's generic failure (missing credential, cancel, or timeout) for you.
