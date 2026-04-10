# Rust

We recommend to add breez sdk as a git dependency with a specific release tag.
Check [breez/spark-sdk](https://github.com/breez/spark-sdk/releases) for the latest version.

```toml
[dependencies]
breez-sdk-spark = { git = "https://github.com/breez/spark-sdk", tag = "{VERSION}" }
```

## Example App

For a full working example app, see the [Rust CLI example app](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/cli).
