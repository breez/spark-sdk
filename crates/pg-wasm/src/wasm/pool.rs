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

use std::ops::Deref;
use std::rc::Rc;

use super::client::Client;
use super::error::Error;
use super::js::{create_pool, JsPool};

/// Shared, multi-checkout connection pool. Cheap to clone (it's an `Rc`
/// around a JS handle).
#[derive(Clone)]
pub struct Pool {
    inner: Rc<JsPool>,
}

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

impl Deref for Object {
    type Target = Client;

    fn deref(&self) -> &Client {
        &self.client
    }
}

// We deliberately don't impl `DerefMut`. `Client::query` and friends
// take `&self`; the only `&mut self` method is `Client::transaction`. If
// the SDK needs `tx = pool.get().await?.transaction().await?`, we'll
// add a passthrough `Object::transaction` then — the `&mut Client` it
// requires is straightforward to expose from `Object` once a concrete
// caller appears.
