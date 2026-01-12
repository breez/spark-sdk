# Advanced features

The SDK supports advanced features that may be useful in specific use cases:

- **[Using an External Signer](external_signer.md)** provides custom signing logic and enables integrating with hardware wallets, MPC protocols, or existing wallet infrastructure
- **[Spark HTLC payments](htlcs.md)** enable conditional payments and are useful for implementing atomic cross-chain swaps
- **[Custom leaf optimization](optimize.md)** allows defining the leaf optimization policy and controlling when it occurs in order to minimize payment latency
- **[Seedless restore](seedless_restore.md)** enables wallet recovery using passkeys with WebAuthn PRF extension, eliminating the need to backup mnemonic phrases
