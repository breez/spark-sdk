//! `wasm-bindgen` extern bindings to `js/pg-wasm-bridge.cjs`.
//!
//! The bridge owns the `pg.Client` lifecycle and the Submittable that
//! implements the Postgres extended-query protocol round-trip via
//! `pg-protocol`. Rust receives binary buffers; trait-side decoding via
//! `FromSql` happens after the boundary crossing.

use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/js/pg-wasm-bridge.cjs")]
extern "C" {
    /// Opaque handle to a connected `pg.Client` on the JS side. The
    /// underlying client may be standalone (from `connect_client`) or
    /// checked out from a pool (from `JsPool::connect`); the JS class
    /// knows which and dispatches `close()` accordingly.
    pub type JsClient;

    /// Opaque handle to the result of one `queryBinary` round-trip.
    pub type JsQueryResult;

    /// Opaque handle to a JS-side `pg.Pool`.
    pub type JsPool;

    /// Open a connection. Equivalent to constructing `new pg.Client({
    /// connectionString })` and `await client.connect()`.
    #[wasm_bindgen(js_name = "connectClient", catch)]
    pub async fn connect_client(connection_string: &str) -> Result<JsClient, JsValue>;

    /// Construct a pool from a libpq connection string.
    #[wasm_bindgen(js_name = "createPool", catch)]
    pub fn create_pool(connection_string: &str) -> Result<JsPool, JsValue>;

    /// Check a client out of the pool.
    #[wasm_bindgen(method, catch)]
    pub async fn connect(this: &JsPool) -> Result<JsClient, JsValue>;

    /// Best-effort pool close.
    #[wasm_bindgen(method, js_name = "end")]
    pub fn end(this: &JsPool);

    /// Send a Parse + Bind + Execute + Sync sequence. Parameters are
    /// passed as binary-encoded `Uint8Array`s; pg-protocol auto-marks each
    /// `Buffer` value with the BINARY format code. Results are requested
    /// in binary format too. `param_oids` is parameter type OIDs; `0`
    /// means unspecified (server will infer).
    #[wasm_bindgen(method, catch, js_name = "queryBinary")]
    pub async fn query_binary(
        this: &JsClient,
        text: &str,
        statement_name: &str,
        param_oids: Vec<u32>,
        param_values: js_sys::Array,
    ) -> Result<JsQueryResult, JsValue>;

    /// Run one or more SQL statements via the simple query protocol.
    /// Used for `BEGIN` / `COMMIT` / `ROLLBACK` and `batch_execute`.
    #[wasm_bindgen(method, catch, js_name = "simpleQuery")]
    pub async fn simple_query(this: &JsClient, sql: &str) -> Result<(), JsValue>;

    /// Best-effort cleanup: for a pooled client, releases back to the
    /// pool; for a standalone client, ends the TCP connection. Fire-
    /// and-forget; safe to call from `Drop`.
    #[wasm_bindgen(method, js_name = "close")]
    pub fn close(this: &JsClient);

    // ── JsQueryResult accessors ──────────────────────────────────────────

    #[wasm_bindgen(method, getter, js_name = "fieldOids")]
    pub fn field_oids(this: &JsQueryResult) -> js_sys::Uint32Array;

    #[wasm_bindgen(method, getter, js_name = "fieldNames")]
    pub fn field_names(this: &JsQueryResult) -> js_sys::Array;

    /// `Array<Array<Uint8Array|null>>` — outer array is rows, inner is
    /// per-column binary-encoded value or `null`.
    #[wasm_bindgen(method, getter, js_name = "rows")]
    pub fn rows(this: &JsQueryResult) -> js_sys::Array;

    /// `CommandComplete` row-count tag, if any. Stored as `f64` for
    /// wasm-bindgen ABI compatibility; callers cast to `u64`.
    #[wasm_bindgen(method, getter, js_name = "rowsAffected")]
    pub fn rows_affected(this: &JsQueryResult) -> Option<f64>;
}
