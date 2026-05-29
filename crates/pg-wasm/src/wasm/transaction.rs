//! `Transaction` — wraps a `Client` for the duration of a `BEGIN` /
//! `COMMIT` (or `ROLLBACK`) block.
//!
//! Mirrors `tokio_postgres::Transaction`. The lifetime is tied to the
//! borrowing `Client`; if the `Transaction` is dropped without an
//! explicit `commit()` or `rollback()`, we fire a best-effort
//! `ROLLBACK` via [`wasm_bindgen_futures::spawn_local`] so the
//! connection isn't left in an open-transaction state.

use postgres_types::{ToSql, Type};
use wasm_bindgen_futures::spawn_local;

use super::client::Client;
use super::error::Error;
use super::row::Row;
use super::statement::{Statement, ToStatement};

/// In-progress transaction. Calling `commit` or `rollback` consumes the
/// transaction; otherwise its `Drop` schedules a `ROLLBACK`.
pub struct Transaction<'a> {
    client: &'a Client,
    finished: bool,
}

unsafe impl<'a> Send for Transaction<'a> {}
unsafe impl<'a> Sync for Transaction<'a> {}

impl<'a> Transaction<'a> {
    pub(crate) fn new(client: &'a Client) -> Self {
        Self {
            client,
            finished: false,
        }
    }

    /// Commit the transaction.
    pub async fn commit(mut self) -> Result<(), Error> {
        self.finished = true;
        self.client.batch_execute("COMMIT").await
    }

    /// Roll back the transaction explicitly.
    pub async fn rollback(mut self) -> Result<(), Error> {
        self.finished = true;
        self.client.batch_execute("ROLLBACK").await
    }

    // ── query delegation ────────────────────────────────────────────────

    pub async fn prepare(&self, sql: &str) -> Result<Statement, Error> {
        self.client.prepare(sql).await
    }

    pub async fn prepare_typed(
        &self,
        sql: &str,
        param_types: &[Type],
    ) -> Result<Statement, Error> {
        self.client.prepare_typed(sql, param_types).await
    }

    pub async fn query<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, Error>
    where
        T: ?Sized + ToStatement,
    {
        self.client.query(statement, params).await
    }

    pub async fn query_one<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Row, Error>
    where
        T: ?Sized + ToStatement,
    {
        self.client.query_one(statement, params).await
    }

    pub async fn query_opt<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, Error>
    where
        T: ?Sized + ToStatement,
    {
        self.client.query_opt(statement, params).await
    }

    pub async fn execute<T>(
        &self,
        statement: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<u64, Error>
    where
        T: ?Sized + ToStatement,
    {
        self.client.execute(statement, params).await
    }

    pub async fn batch_execute(&self, sql: &str) -> Result<(), Error> {
        self.client.batch_execute(sql).await
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        // We can't await in Drop. Spawn the rollback on the JS event loop.
        // The `Client` is `Clone` (Rc), so we hand a fresh handle to the
        // spawned task — it outlives this Transaction.
        let client = self.client.clone();
        spawn_local(async move {
            // If this fails the connection is probably already torn down;
            // there's nothing useful to surface to anyone at this point.
            let _ = client.batch_execute("ROLLBACK").await;
        });
    }
}
