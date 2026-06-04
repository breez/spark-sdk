# pg-wasm

A [`tokio-postgres`][tp]-shaped PostgreSQL client that compiles for
`wasm32-unknown-unknown` by dispatching queries through [node-postgres][pg]
over the `wasm-bindgen` boundary.

[tp]: https://docs.rs/tokio-postgres
[pg]: https://www.npmjs.com/package/pg

```toml
[dependencies]
pg-wasm = "0.1"
```

```javascript
// In the host Node project
npm install pg@^8.18.0
```

## Why

`tokio-postgres` connects to Postgres over a raw TCP socket via tokio's
`net` feature. That feature doesn't compile on `wasm32-unknown-unknown`:
no syscalls, no sockets. Existing workarounds rely on a host-supplied
TCP API (`worker-rs` on Cloudflare Workers, WASI sockets on WasmEdge),
which isn't available in the plain Node-via-`wasm-bindgen` environment.

`pg-wasm` doesn't speak the Postgres wire protocol from Rust on wasm.
Instead it hands binary-encoded parameters across the `wasm-bindgen`
boundary to a Node.js `pg.Client`, which speaks the protocol over a
real TCP socket. From the caller's perspective the API looks like
`tokio-postgres`; under the hood it's a JS bridge.

## What you get

The wasm target exposes a focused subset of the `tokio-postgres` and
`deadpool-postgres` API:

| Type | Methods |
|---|---|
| `Client` | `prepare`, `prepare_typed`, `query`, `query_one`, `query_opt`, `execute`, `batch_execute`, `transaction` |
| `Transaction<'_>` | same query/execute methods plus `commit` / `rollback`; rolls back on `Drop` if neither was called |
| `Pool` | `get -> Object` (derefs to `Client`); checked-out clients return to the pool on `Drop` |
| `Row` | `get<T: FromSql>`, `try_get`, `columns`, `len` |
| `Statement` | cheap to clone; cached per-client by SQL text |
| `Error` | `code() -> Option<&str>` (SQLSTATE), `is_closed()`, `as_db_error()` |
| `types` | re-export of [`postgres-types`][pt]; `ToSql` / `FromSql` impls from `chrono`, `uuid`, `serde_json`, `rust_decimal`, etc. work unchanged |

[pt]: https://docs.rs/postgres-types

Both parameters and result columns travel in Postgres binary format,
so the full `ToSql` / `FromSql` ecosystem applies. The `&str` overload
of `query` / `execute` does a server-side `Describe Statement`
round-trip on first use to fetch inferred parameter type OIDs, matching
`tokio_postgres::Client::query("...", &[...])`'s behavior.

## Quick start

```rust
use pg_wasm::{types::Type, Pool};

#[wasm_bindgen]
pub async fn balance(user_id: Vec<u8>) -> Result<i64, JsValue> {
    let pool = Pool::new("postgres://user:pass@host:5432/db")
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    let client = pool.get().await
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    let stmt = client
        .prepare_typed("SELECT balance_sats FROM accounts WHERE user_id = $1", &[Type::BYTEA])
        .await
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    let row = client.query_one(&stmt, &[&user_id]).await
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(row.get(0))
}
```

Transactions follow the standard pattern:

```rust
let mut client = pool.get().await?;
let tx = client.transaction().await?;
tx.execute("INSERT INTO ledger (user_id, delta) VALUES ($1, $2)", &[&user_id, &delta]).await?;
tx.execute("UPDATE accounts SET balance_sats = balance_sats + $2 WHERE user_id = $1", &[&user_id, &delta]).await?;
tx.commit().await?;
```

## How it works

1. Rust serializes each parameter to Postgres binary wire format via
   `postgres_types::ToSql::to_sql_checked` into a `BytesMut`.
2. Each buffer crosses the `wasm-bindgen` boundary as a `Uint8Array`,
   along with the parameter type OIDs.
3. On the JS side, the bridge implements
   [node-postgres' `Submittable` interface][submittable]: it cooperates
   with the client's query queue, sends `Parse + Bind + Describe +
   Execute + Sync` frames using `pg-protocol`'s serializer, and pulls
   the response back through the same `Connection` event stream
   node-postgres uses internally.
4. `pg-protocol`'s parser decodes every column value as a UTF-8 string
   by default, which loses arbitrary bytes. To preserve them the
   bridge patches `Parser.prototype.handlePacket` to handle `DataRow`
   (code `0x44`) itself, reading column bytes via `reader.bytes()`.
   The patch is gated on a per-connection flag, so coexisting
   `pg.Client` and `pg.Pool` instances elsewhere in the process are
   unaffected.
5. Result `Uint8Array`s come back across the boundary and are decoded
   on demand via `postgres_types::FromSql::from_sql`.

Prepared statement names are stable SipHashes of the SQL text
(`pgw_<hex>`). This is important because `pg.Pool` recycles its
underlying `pg.Client` instances: a counter that resets per Rust
`Client` wrapper would silently collide with cached names from a
previous checkout, and `Describe Statement` would return the previous
statement's parameter shape. Stable hashes sidestep the problem.

[submittable]: https://node-postgres.com/apis/client#submittable

## Native targets

On non-wasm targets, `pg-wasm` is a thin pass-through:

```rust
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub use tokio_postgres::*;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub mod pool {
    pub use deadpool_postgres::*;
}
```

This lets the same call sites compile against either real
`tokio-postgres` (native) or this crate's wasm bridge, with no `cfg`
in your application code. The `types` module re-exports
[`postgres-types`][pt] on both targets, so `ToSql` / `FromSql` trait
identity is preserved across the boundary.

## Compatibility

- **`pg`**: `^8.18` (the bridge patches `pg-protocol` internals; pin a
  major version in your `package.json`).
- **Node.js**: `>= 18`.
- **Rust**: `2024` edition; MSRV `1.88`.

## Limitations

- **Node.js only.** Browsers can't open raw TCP connections, so the
  bridge can't function there. Using `pg-wasm` from a browser bundle
  requires excluding it from the build entirely (e.g. behind a Cargo
  feature): the `pg-wasm-bridge.cjs` snippet calls Node-only modules.

- **Subset of `tokio-postgres`.** No `COPY` (`copy_in` / `copy_out`),
  no `LISTEN` / `NOTIFY`, no `query_raw` row streaming, no
  cancellation token, no custom TLS configuration from Rust. TLS is
  handled by `node-postgres` on the JS side via the `sslmode` parameter
  in your connection string.

- **Patches `pg-protocol` internals.** The DataRow binary-preserving
  patch reaches into `pg-protocol/dist/parser.js` and
  `pg/lib/connection.js` via `require.cache`. A breaking change to
  either is detected at runtime and surfaces as a clear error, but
  consumers should pin pg's major version.

- **Statement names hash SQL.** Two SQL strings whose SipHash collides
  would conflict on a shared connection. The collision probability for
  the ~100 statements a typical SDK uses is astronomically low; the
  bridge has a defensive `parsedStatements[name] === text` check that
  throws loudly if one ever occurs.

## License

MIT
