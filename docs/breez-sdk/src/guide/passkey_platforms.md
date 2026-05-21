# Platform-specific notes

## Domain association diagnostic

{{#name check_domain_association}} performs an Apple-app-site-association probe (iOS) or Digital Asset Links lookup (Android) against the configured RP and returns a typed {{#name DomainAssociation}}: `Associated`, `NotAssociated { source, reason }`, or `Skipped { reason }`. Use it to surface configuration mistakes (entitlement missing, AASA file not deployed) before a WebAuthn ceremony fails opaquely. Most hosts reach this through {{#name PasskeyClient.check_availability}}, which folds it together with `is_supported` into one call; use the diagnostic directly when you need only the association signal:

{{#tabs passkey:domain-association}}

## Capacitor plugins

For Capacitor apps, the JS layer talks to the iOS Swift / Android Kotlin native plugin through Capacitor's bridge. The SDK ships a TypeScript-only contract sub-export so plugin authors can keep their `definitions.ts` in lockstep with the canonical native shape:

```ts
import type {
  PasskeyPrfPlugin,
  DomainAssociation,
} from '@breeztech/breez-sdk-spark/passkey-capacitor-bridge';

import { registerPlugin } from '@capacitor/core';

export const PasskeyPrf =
  registerPlugin<PasskeyPrfPlugin>('PasskeyPrf');
```

Mirror the contract on both sides. The interface matches the canonical iOS `PasskeyAssertionCore` and Android `CredentialManagerPrfCore` plugin surface bundled with the SDK. `Uint8Array` values are exchanged as base64-url-safe strings (no padding), since Capacitor's bridge cannot transport binary data directly.

## Platform considerations

- **Web (browsers)**: Use the WebAuthn API with the `prf` extension. Browsers handle the salt transformation internally. Use discoverable credentials (`residentKey: 'required'`) with empty `allowCredentials` for assertion so the browser discovers the credential by RP ID.
- **Android / iOS**: Use native passkey APIs with PRF support. Ensure the Associated Domains / Asset Links configuration is in place for `keys.breez.technology`.
- **CLI / Desktop (CTAP2)**: Use the `hmac-secret` extension directly. Non-browser implementations must apply the WebAuthn salt transformation manually to produce the same PRF output as browsers:

  ```
  actualSalt = SHA-256("WebAuthn PRF" || 0x00 || developerSalt)
  ```

  This transformation is defined in the <a target="_blank" href="https://w3c.github.io/webauthn/#prf-extension">W3C WebAuthn PRF extension spec</a> and ensures that the same passkey + salt produces identical seeds across browser and native implementations.
