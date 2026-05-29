//! `Client` — owns one `pg.Client` checkout and dispatches queries
//! through the JS bridge.
//!
//! v0 covers `prepare` / `prepare_typed` / `query` / `query_one` /
//! `query_opt` / `execute` / `batch_execute`. Transactions and pooling
//! land in follow-up modules.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use postgres_types::{IsNull, ToSql, Type};
use wasm_bindgen::{JsCast, JsValue};

use super::error::Error;
use super::js::{JsClient, JsQueryResult};
use super::row::{Column, Row};
use super::statement::{Statement, StmtCache, ToStatement};
use super::transaction::Transaction;

/// A connection to a Postgres backend.
///
/// Each `Client` owns one `pg.Client` on the JS side. Cloning is cheap
/// (it's an `Rc` around the JS handle) but **does not** create a second
/// backend connection — clones share the same connection and the same
/// statement cache. To get an independent connection use [`super::connect`]
/// again (or, later, the pool).
///
/// Dropping the last clone fires `client.end()` on the JS side.
#[derive(Clone)]
pub struct Client {
    inner: Rc<ClientInner>,
}

struct ClientInner {
    js: JsClient,
    stmt_cache: RefCell<StmtCache>,
}

impl Drop for ClientInner {
    fn drop(&mut self) {
        // For a standalone client this calls `pg.Client.end()`; for a
        // pooled client it calls `release()`. The JS-side `JsClient`
        // wrapper knows which kind it is and dispatches accordingly.
        // Fire-and-forget — safe in `Drop`.
        self.js.close();
    }
}

impl Client {
    pub(crate) fn new(js: JsClient) -> Self {
        Self {
            inner: Rc::new(ClientInner {
                js,
                stmt_cache: RefCell::new(StmtCache::default()),
            }),
        }
    }

    /// Prepare a statement and let the server infer parameter types.
    ///
    /// On a first call this round-trips Parse + Describe Statement +
    /// Sync to fetch the inferred parameter type OIDs, then caches the
    /// resulting [`Statement`] by SQL text. Subsequent calls with the
    /// same SQL hit the cache and skip the round-trip.
    pub async fn prepare(&self, sql: &str) -> Result<Statement, Error> {
        if let Some(stmt) = self.inner.stmt_cache.borrow().get(sql) {
            return Ok(stmt);
        }
        let name = self.inner.stmt_cache.borrow_mut().intern(sql);
        let prep = self
            .inner
            .js
            .prepare_statement(sql, &name)
            .await
            .map_err(Error::from_js)?;
        let oids = prep.prepare_param_oids();
        let mut param_types = Vec::with_capacity(oids.length() as usize);
        for i in 0..oids.length() {
            let oid = oids.get_index(i);
            param_types.push(Type::from_oid(oid).unwrap_or(Type::UNKNOWN));
        }
        let stmt = Statement::new(name, sql.to_string(), param_types);
        self.inner
            .stmt_cache
            .borrow_mut()
            .store(sql, stmt.clone());
        Ok(stmt)
    }

    /// Prepare a statement with explicit parameter types. No server
    /// round-trip; the caller asserts the types match the SQL. Useful
    /// when you already know them (most SDK queries) and want to skip
    /// the Describe Statement.
    pub async fn prepare_typed(
        &self,
        sql: &str,
        param_types: &[Type],
    ) -> Result<Statement, Error> {
        let name = self.inner.stmt_cache.borrow_mut().intern(sql);
        Ok(Statement::new(name, sql.to_string(), param_types.to_vec()))
    }

    /// Execute a query and collect all rows.
    ///
    /// `statement` can be either a `&Statement` (already prepared) or a
    /// `&str` (auto-prepared on first use), matching
    /// `tokio_postgres::Client::query`'s signature.
    pub async fn query<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, Error>
    where
        T: ?Sized + ToStatement,
    {
        let stmt = statement.__to_statement(self).await?;
        let result = self.exec(&stmt, params).await?;
        Ok(decode_rows(&result))
    }

    /// Execute a query expecting exactly one row.
    pub async fn query_one<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Row, Error>
    where
        T: ?Sized + ToStatement,
    {
        let mut rows = self.query(statement, params).await?;
        match rows.len() {
            1 => Ok(rows.pop().expect("len checked")),
            n => Err(Error::row_count(n)),
        }
    }

    /// Execute a query expecting zero or one rows.
    pub async fn query_opt<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, Error>
    where
        T: ?Sized + ToStatement,
    {
        let mut rows = self.query(statement, params).await?;
        match rows.len() {
            0 => Ok(None),
            1 => Ok(Some(rows.pop().expect("len checked"))),
            n => Err(Error::row_count(n)),
        }
    }

    /// Execute a statement, returning the number of rows affected.
    pub async fn execute<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<u64, Error>
    where
        T: ?Sized + ToStatement,
    {
        let stmt = statement.__to_statement(self).await?;
        let result = self.exec(&stmt, params).await?;
        Ok(result.rows_affected().map_or(0, |f| f as u64))
    }

    /// Execute one or more statements via the simple query protocol.
    /// Used for `BEGIN` / `COMMIT` / `ROLLBACK` and ad-hoc DDL.
    pub async fn batch_execute(&self, sql: &str) -> Result<(), Error> {
        self.inner
            .js
            .simple_query(sql)
            .await
            .map_err(Error::from_js)
    }

    /// Start a transaction by sending `BEGIN`. The returned
    /// [`Transaction`] borrows this client for the duration of the
    /// transaction; dropping it without an explicit `commit()` or
    /// `rollback()` schedules a best-effort `ROLLBACK`.
    pub async fn transaction(&mut self) -> Result<Transaction<'_>, Error> {
        self.batch_execute("BEGIN").await?;
        Ok(Transaction::new(self))
    }

    // ── inner dispatch ──────────────────────────────────────────────────

    async fn exec(
        &self,
        stmt: &Statement,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<JsQueryResult, Error> {
        let (oids, js_values) = encode_params(stmt, params)?;
        self.inner
            .js
            .query_binary(stmt.sql(), stmt.name(), oids, js_values)
            .await
            .map_err(Error::from_js)
    }
}

/// Encode each `ToSql` parameter to its binary wire form via
/// `postgres-types`, then copy into a JS `Array<Uint8Array | null>` along
/// with the matching OID list. The copy into `Uint8Array` is intentional:
/// pg-protocol's auto-detection only marks `Buffer` values as binary, and
/// we need fresh JS-owned bytes that survive any subsequent wasm memory
/// growth on the JS side too.
fn encode_params(
    stmt: &Statement,
    params: &[&(dyn ToSql + Sync)],
) -> Result<(Vec<u32>, js_sys::Array), Error> {
    let mut oids = Vec::with_capacity(params.len());
    let js_values = js_sys::Array::new_with_length(params.len() as u32);
    let mut buf = BytesMut::with_capacity(64);

    for (i, param) in params.iter().enumerate() {
        buf.clear();
        let ty = stmt.param_type(i).unwrap_or(&Type::UNKNOWN);
        oids.push(ty.oid());

        let is_null = param.to_sql_checked(ty, &mut buf).map_err(Error::encode)?;
        match is_null {
            IsNull::No => {
                let u8a = js_sys::Uint8Array::new_with_length(buf.len() as u32);
                u8a.copy_from(&buf);
                js_values.set(i as u32, u8a.into());
            }
            IsNull::Yes => js_values.set(i as u32, JsValue::NULL),
        }
    }

    Ok((oids, js_values))
}

/// Convert the JS-side query result into an owned `Vec<Row>`.
fn decode_rows(result: &JsQueryResult) -> Vec<Row> {
    let oids_arr = result.field_oids();
    let names_arr = result.field_names();
    let n_cols = oids_arr.length();

    let mut columns_vec = Vec::with_capacity(n_cols as usize);
    for i in 0..n_cols {
        let oid = oids_arr.get_index(i);
        let name = names_arr.get(i).as_string().unwrap_or_default();
        let ty = Type::from_oid(oid).unwrap_or(Type::UNKNOWN);
        columns_vec.push(Column { name, ty });
    }
    let columns: Arc<[Column]> = columns_vec.into();

    let js_rows = result.rows();
    let row_count = js_rows.length();
    let mut out = Vec::with_capacity(row_count as usize);

    for i in 0..row_count {
        let row_arr: js_sys::Array = js_rows.get(i).unchecked_into();
        let mut values: Vec<Option<Bytes>> = Vec::with_capacity(columns.len());

        for j in 0..columns.len() as u32 {
            let v = row_arr.get(j);
            if v.is_null() || v.is_undefined() {
                values.push(None);
            } else {
                let u8a: js_sys::Uint8Array = v.unchecked_into();
                let mut buf = vec![0u8; u8a.length() as usize];
                u8a.copy_to(&mut buf);
                values.push(Some(Bytes::from(buf)));
            }
        }

        out.push(Row::new(columns.clone(), values));
    }

    out
}
