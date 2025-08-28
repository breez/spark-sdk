# Introduction

The SDK docs are live at [https://sdk-doc-spark.breez.technology](https://sdk-doc-spark.breez.technology).

## Contributions

For syntax and supported features, see [https://rust-lang.github.io/mdBook](https://rust-lang.github.io/mdBook).

## Develop

To locally serve the docs run:

```bash
cargo install mdbook
cargo install --path ./snippets-processor
cargo install mdbook-variables
mdbook build
mdbook serve --open

```
