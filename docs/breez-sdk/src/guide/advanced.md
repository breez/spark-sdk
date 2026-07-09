# Advanced features

The SDK supports advanced features that may be useful in specific use cases:

- **[Custom configuration](config.md)** enables fine-tuning the SDK behavior with various configuration options
- **[Custom leaf optimization](optimize.md)** allows defining the leaf optimization policy and controlling when it occurs in order to minimize payment latency
- **[Conditional payments](htlcs.md)** are useful for implementing atomic cross-chain swaps
- **[Using an External Signer](external_signer.md)** provides custom signing logic and enables integrating with hardware wallets, MPC protocols, or existing wallet infrastructure
- **[Server mode](server_mode.md)** is the SDK profile for multi-tenant server deployments where each request builds an ephemeral SDK and the host orchestrates sync, claiming, and event delivery explicitly
- **[Client signing](client_signing.md)** lets a server drive payments while the key that approves them stays with the user, who reviews and signs each payment on their side
- **[Send USDC/USDT](cross_chain.md)** to a recipient on an external chain
