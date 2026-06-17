mod chain_service;
mod error;
mod event;
mod issuer;
mod logger;
mod models;
mod passkey;
mod persist;
mod sdk;
mod sdk_builder;
mod sdk_context;
mod signer;
mod token_store;
mod tree_store;
mod turnkey;

use wasm_bindgen::prelude::wasm_bindgen;

/// Runs automatically when the wasm module is instantiated. Installs the
/// panic hook so Rust panics surface as readable `console.error` output
/// (with the panic message + `file.rs:line`) instead of a bare
/// `RuntimeError: unreachable`.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}
