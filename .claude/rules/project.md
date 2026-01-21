# Breez SDK - Nodeless (Spark Implementation)

End-to-end solution for integrating self-custodial Bitcoin payments into apps and services. Uses Spark, a Bitcoin-native Layer 2 built on shared signing, to enable real-time, low-fee payments.

## Key Features

- Send/receive via Lightning, LNURL-pay, Lightning addresses, Bolt11, BTC addresses, Spark addresses
- Issue, send, and receive Spark tokens (BTKN)
- On-chain interoperability with automatic claims
- Multi-app and multi-device support via real-time sync
- Keys held only by users (self-custodial)

## Platform Bindings

| Language | Package |
|----------|---------|
| Rust | `crates/breez-sdk/core` |
| Swift | `crates/breez-sdk/bindings` |
| Kotlin | `crates/breez-sdk/bindings` |
| Go | `crates/breez-sdk/bindings` |
| Python | `crates/breez-sdk/bindings` |
| React Native | `crates/breez-sdk/bindings` |
| C# | `crates/breez-sdk/bindings` |
| WASM | `crates/breez-sdk/wasm` |
| Flutter | `packages/flutter` |

## References

- [SDK Documentation](https://sdk-doc-spark.breez.technology/)
- [API Documentation](https://breez.github.io/spark-sdk/breez_sdk_spark/index.html)
- Build commands: See `./CLAUDE.md`
