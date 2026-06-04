//! `Pool` — JS-backed connection pool mimicking
//! [`deadpool_postgres::Pool`]'s surface.
//!
//! The actual pool lives on the JS side as a `pg.Pool`. `Pool::get` does
//! `pool.connect()`, hands the resulting checked-out `pg.Client` to a
//! Rust [`Client`], and wraps it in an [`Object`]. The JS-side
//! [`JsClient`](super::js::JsClient) wrapper carries a `pooled` flag
//! that decides whether dropping the inner `pg.Client` should
//! `release()` it back to the pool or `end()` the TCP connection.
//!
//! Binary DataRow handling is wired into the pool by subclassing
//! `pg.Client` in `pg-wasm-bridge.cjs`. That subclass sets
//! `connection.__pgWasmBinaryDataRow = true` in its constructor, so
//! every client minted by the pool — including lazily-created ones —
//! picks up the binary DataRow parser automatically.

use std::ops::{Deref, DerefMut};
use std::rc::Rc;

use super::client::Client;
use super::error::Error;
use super::js::{create_pool, JsPool};

/// Alias matching the `deadpool_postgres::PoolError` name. Pool errors
/// in pg-wasm are surfaced through the unified `Error` type — there's no
/// separate `PoolError` enum on wasm because the JS-side pool surfaces a
/// single `JsValue` we wrap as `Error::Io`. Code that imports
/// `pg_wasm::pool::PoolError` resolves to the same `Error` type so
/// signatures match across the two backends.
pub type PoolError = Error;

/// Shared, multi-checkout connection pool. Cheap to clone (it's an `Rc`
/// around a JS handle).
///
/// Pool is declared `Send + Sync` via `unsafe impl`s for parity with
/// `deadpool_postgres::Pool`, which the SDK's storage layer stores
/// inside `Send + Sync` structs. Wasm is single-threaded, the JS handle
/// is process-local, and shared-ownership tracking happens through
/// `Rc`'s non-atomic refcounter — all of which is sound here only
/// because of the single-threaded environment. The same `unsafe impl`
/// pattern is used by other wasm SDK types (e.g. `WasmStorage`).
#[derive(Clone)]
pub struct Pool {
    inner: Rc<JsPool>,
}

unsafe impl Send for Pool {}
unsafe impl Sync for Pool {}

impl Pool {
    /// Build a pool from a libpq connection string.
    ///
    /// v0 leaves all sizing/timing config at the JS-side defaults (the
    /// `pg.Pool` defaults: `max=10`, no explicit timeouts). Finer
    /// control can be added when the SDK needs it; the surface mirrors
    /// `deadpool_postgres::Pool` so a future `Config` struct slots in.
    pub fn new(connection_string: &str) -> Result<Self, Error> {
        let js = create_pool(connection_string).map_err(Error::from_js)?;
        Ok(Self { inner: Rc::new(js) })
    }

    /// Check out a connection. Drop the returned [`Object`] to release.
    pub async fn get(&self) -> Result<Object, Error> {
        let js_client = self.inner.connect().await.map_err(Error::from_js)?;
        Ok(Object {
            client: Client::new(js_client),
        })
    }

    /// Close the pool (and all idle clients) best-effort. Outstanding
    /// checkouts continue to function until they're dropped.
    pub fn close(&self) {
        self.inner.end();
    }
}

/// A checked-out connection. Derefs to [`Client`] so query methods are
/// directly callable: `pool.get().await?.query(...)`. When dropped, the
/// underlying `pg.Client` returns to the pool via its `release()` method
/// — see `JsClient.close` in the JS bridge.
pub struct Object {
    client: Client,
}

unsafe impl Send for Object {}
unsafe impl Sync for Object {}

impl Deref for Object {
    type Target = Client;

    fn deref(&self) -> &Client {
        &self.client
    }
}

impl DerefMut for Object {
    fn deref_mut(&mut self) -> &mut Client {
        &mut self.client
    }
}
