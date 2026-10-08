# Advanced features

The SDK supports advanced features that may be useful in specific use cases:

- **[Custom configuration](config.md)** enables fine-tuning the SDK behavior with various configuration options
- **[SOCKS5 proxy](proxy.md)** sends the connections the SDK opens through a SOCKS5 proxy, such as a local Tor daemon
- **[Custom leaf optimization](optimize.md)** allows defining the leaf optimization policy and controlling when it occurs in order to minimize payment latency
- **[Conditional payments](htlcs.md)** are useful for implementing atomic cross-chain swaps
- **[Using an External Signer](external_signer.md)** provides custom signing logic and enables integrating with hardware wallets, MPC protocols, or existing wallet infrastructure
- **[Managing webhooks](webhooks.md)** delivers real-time notifications of wallet events, such as completed payments or on-chain deposits, to a URL you register
- **[Unilateral exit](unilateral_exit.md)** moves funds onto the Bitcoin blockchain without the Spark operators, as a safety net

If you're running the SDK on a server, see [Running on a server](server_deployments.md).