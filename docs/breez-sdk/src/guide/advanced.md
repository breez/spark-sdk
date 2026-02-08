# Advanced features

The SDK supports advanced features that may be useful in specific use cases:

- **[Custom configuration](config.md)** enables fine-tuning the SDK behavior with various configuration options
- **[Custom leaf optimization](optimize.md)** allows defining the leaf optimization policy and controlling when it occurs in order to minimize payment latency
- **[Spark HTLC payments](htlcs.md)** enable conditional payments and are useful for implementing atomic cross-chain swaps
- **[Stable balance](stable_balance.md)** automatically converts received Bitcoin to a stable token, protecting against price volatility while still accepting Bitcoin payments
- **[Using an External Signer](external_signer.md)** provides custom signing logic and enables integrating with hardware wallets, MPC protocols, or existing wallet infrastructure
