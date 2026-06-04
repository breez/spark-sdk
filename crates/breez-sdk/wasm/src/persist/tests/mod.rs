#[cfg(not(feature = "browser-tests"))]
mod node;

// pg-wasm tests are Node-only (the bridge requires CommonJS pg).
#[cfg(all(not(feature = "browser-tests"), feature = "postgres"))]
mod pg_wasm_smoke;

#[cfg(all(not(feature = "browser-tests"), feature = "postgres"))]
mod pg_wasm_rust_storage;

#[cfg(not(feature = "browser-tests"))]
mod mysql;

#[cfg(not(feature = "browser-tests"))]
mod mysql_foreign_keys;

#[cfg(feature = "browser-tests")]
mod web;
