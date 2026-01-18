# Updating SDK Interfaces

When changing the SDK's public interface, update these files:

1. **crates/breez-sdk/core/src/models.rs** - Add UniFFI macros to interface types
2. **crates/breez-sdk/wasm/src/models.rs** - Update exported structs/enums
3. **crates/breez-sdk/wasm/src/sdk.rs** - Update WASM interface
4. **packages/flutter/rust/src/models.rs** - Update mirrored structs/enums
5. **packages/flutter/rust/src/sdk.rs** - Update Flutter interface

Run `.claude/skills/pr-review/validate-bindings.sh` to verify consistency.
