# Development guide

## Repository structure

### Crates

The project is organized into several Rust crates:

- **breez-sdk**: Core functionality of the SDK divided into sub-crates:
  - `bindings`: Contains UniFFI bindings for multiple languages (Kotlin, Swift, etc.)
  - `cli`: A command-line interface for testing and interacting with the SDK
  - `common`: Shared utilities and components used across other crates
  - `core`: Core functionality of the Breez SDK
  - `wasm`: WebAssembly bindings for JavaScript/TypeScript environments

- **macros**: Contains procedural macros used throughout the project:
  - `async_trait.rs`: Macros for working with async traits
  - `derive_from.rs`: Macros for deriving From implementations
  - `testing.rs`: Macros for testing utilities

- **spark**: The core implementation of the Spark protocol
  - Contains protocol definitions, schema, and business logic

- **spark-itest**: Integration tests for the Spark implementation
  - Includes Docker configurations for testing in isolated environments

- **spark-wallet**: Implementation of a wallet using the Spark protocol

- **xtask**: Custom build and development tasks as Rust code
  - Provides commands for building, testing, and formatting the codebase

### Packages

The packages directory contains non-Rust components of the project:

- **wasm**: JavaScript/TypeScript packages derived from the Rust WASM bindings
  - `nodejs`: Node.js-specific package
  - `web`: Browser-specific package
  - `examples`: Example usage of the WASM bindings

## Development workflow

### Setting up your development environment

1. Ensure you have Rust installed with `rustup`:
   ```
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. Clone the repository:
   ```
   git clone https://github.com/breez/spark-sdk.git
   cd spark-sdk
   ```

### Building the project

To build the entire project:
```
make build
```

For a release build:
```
make build-release
```

To build specifically for WASM:
```
make build-wasm
```

### Code formatting and linting

The project enforces code style using rustfmt and clippy. To check your code:

```
make fmt-check     # Check code formatting
make clippy-check  # Check for linting issues
```

To automatically fix formatting and linting issues:

```
make fmt-fix       # Fix code formatting
make clippy-fix    # Fix linting issues where possible
```

You can run all checks at once with:

```
make check         # Runs fmt-check, clippy-check, and tests
```

### Testing

To run all tests:
```
make test
```

For specific test suites:
```
make cargo-test    # Run Rust tests
make wasm-test     # Run WASM tests (both browser and Node.js)
make itest         # Run integration tests
```

## Contributing

Please see [CONTRIBUTING.md](CONTRIBUTING.md) for details on the contribution workflow, pull request process, and code standards.
