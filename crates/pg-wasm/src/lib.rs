//! `pg-wasm` — a [`tokio_postgres`]-shaped client whose `wasm32-unknown-unknown`
//! implementation tunnels Postgres queries through node-postgres on the JS side.
//!
//! # Why
//!
//! `tokio-postgres` opens a raw TCP socket via tokio's `net` feature. That
//! feature does not compile on `wasm32-unknown-unknown` (no syscalls, no
//! sockets). Cloudflare Workers and WasmEdge work around this by providing
//! a host-supplied socket; in the browser/Node-via-wasm-bindgen environment
//! the SDK targets, that escape hatch isn't available.
//!
//! This crate avoids the problem by **not** speaking the Postgres wire
//! protocol from Rust on wasm. Instead, on wasm targets it hands binary-
//! encoded parameters across the wasm-bindgen boundary to a Node.js
//! `pg.Client`, which speaks the wire protocol over a real TCP socket.
//!
//! # Target behaviour
//!
//! * **Non-wasm targets** transparently re-export [`tokio_postgres`] and
//!   [`deadpool_postgres`] (the latter under [`pool`]). The crate is a
//!   zero-cost adapter outside of wasm.
//! * **`wasm32-unknown-unknown`** swaps in a hand-rolled implementation
//!   that mirrors the relevant `tokio-postgres` / `deadpool-postgres`
//!   surface (`Client`, `Row`, `Statement`, `Transaction`, `Pool`) but
//!   dispatches via the JS bridge in `js/pg-wasm-bridge.cjs`.
//!
//! # Types
//!
//! On wasm, [`types`] re-exports the upstream [`postgres_types`] crate —
//! the same crate `tokio_postgres::types` re-exports internally. This
//! preserves `ToSql` / `FromSql` trait identity across targets, so impls
//! from `chrono`, `uuid`, `serde_json`, etc. apply on both sides.

// ── Native: pass-through to tokio-postgres + deadpool-postgres ────────────────

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use tokio_postgres::*;

/// deadpool-postgres pool surface. On wasm this is replaced with a JS-backed
/// pool that has the same shape.
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub mod pool {
    pub use deadpool_postgres::*;
}

// ── Wasm: hand-rolled JS-backed implementation ────────────────────────────────

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
mod wasm;
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub use wasm::*;

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub mod types {
    //! `ToSql`/`FromSql`/`Type` and friends, re-exported from `postgres_types`.
    //!
    //! Trait identity matches `tokio_postgres::types` on native targets, so
    //! downstream impls work on both sides without conditional compilation.
    pub use postgres_types::*;
}
