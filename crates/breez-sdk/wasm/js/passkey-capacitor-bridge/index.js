// This is a TypeScript-only sub-export. There is no runtime here.
// Capacitor plugin authors import the types from
// '@breeztech/breez-sdk-spark/passkey-capacitor-bridge' to keep their
// definitions.ts in sync with the SDK's native plugin contract.
//
// The empty .js exists so module resolvers that require a runtime
// entry alongside the .d.ts (Node ESM, some bundlers) don't fail to
// resolve a sub-export that's pure types.
export {};
