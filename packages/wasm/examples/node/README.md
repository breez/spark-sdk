# Breez SDK Spark - Wasm NodeJs Example

## Prerequisites

Copy the `example.env` file to `.env` and set the environment variables.

## Build

If you are running from a local Wasm package, build the Wasm package first in the [Wasm package](../../) directory and package its dependencies.

```bash
cd ../..
make build
yarn pack
```

Install the dependencies

```bash
npm install
```

## Run

```bash
npm run cli
```
