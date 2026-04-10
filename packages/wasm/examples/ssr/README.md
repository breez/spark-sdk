# Breez SDK — SSR Example

Demonstrates that `@breeztech/breez-sdk-spark` can be imported during
server-side rendering without errors.

## What this tests

1. **Server**: `entry-server.js` imports `connect`, `defaultConfig`, and
   `BreezSdk` from the SDK. These resolve to the SSR stubs — no WASM is
   loaded, no browser or Node.js APIs are touched.

2. **Client**: `entry-client.js` calls `init()` to load the WASM module,
   then calls `defaultConfig()` to prove the SDK works after initialization.

## Running

```bash
npm install
npm run dev
# Open http://localhost:3000
```

If SSR works correctly you will see:
- "SSR succeeded" in the server-rendered section
- "WASM loaded successfully" after the client hydrates
- The regtest config JSON printed below
