//! Prepared statement + per-client cache.
//!
//! Mirrors `tokio_postgres::Statement`. The actual `Parse` frame is sent
//! lazily on first execution; this struct just records the SQL text,
//! the parameter type OIDs, and a stable name for pg-protocol's
//! `parsedStatements` cache to key on.

use std::collections::HashMap;
use std::future::Future;

use postgres_types::Type;

use super::client::Client;
use super::error::Error;

/// Handle to a prepared statement. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Statement {
    name: String,
    sql: String,
    param_types: Vec<Type>,
}

impl Statement {
    pub(crate) fn new(name: String, sql: String, param_types: Vec<Type>) -> Self {
        Self {
            name,
            sql,
            param_types,
        }
    }

    /// Backend-side prepared statement name. Empty string means unnamed.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// SQL text. Always sent on first Parse.
    #[must_use]
    pub fn sql(&self) -> &str {
        &self.sql
    }

    /// Parameter types declared at prepare time. May be empty if the
    /// caller used `prepare` rather than `prepare_typed`.
    #[must_use]
    pub fn params(&self) -> &[Type] {
        &self.param_types
    }

    pub(crate) fn param_type(&self, i: usize) -> Option<&Type> {
        self.param_types.get(i)
    }
}

/// Trait for arguments accepted by [`Client::query`](super::client::Client::query)
/// et al — either a borrowed [`Statement`] (already prepared) or a `&str`
/// (auto-prepared on first use).
///
/// Mirrors `tokio_postgres::ToStatement` so SDK code that uses string
/// literals — `client.execute("SELECT pg_advisory_xact_lock($1)",
/// &[&key]).await?` — works unchanged.
pub trait ToStatement {
    /// Resolve to a `Statement`, preparing if necessary.
    #[doc(hidden)]
    fn __to_statement<'a>(
        &'a self,
        client: &'a Client,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<std::borrow::Cow<'a, Statement>, Error>> + 'a>>;
}

impl ToStatement for Statement {
    fn __to_statement<'a>(
        &'a self,
        _client: &'a Client,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<std::borrow::Cow<'a, Statement>, Error>> + 'a>>
    {
        Box::pin(async move { Ok(std::borrow::Cow::Borrowed(self)) })
    }
}

impl ToStatement for str {
    fn __to_statement<'a>(
        &'a self,
        client: &'a Client,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<std::borrow::Cow<'a, Statement>, Error>> + 'a>>
    {
        Box::pin(async move {
            // Prepare with no explicit parameter types — the server
            // infers from the SQL. This matches `tokio_postgres::Client::query(&str, …)`.
            let stmt = client.prepare(self).await?;
            Ok(std::borrow::Cow::Owned(stmt))
        })
    }
}

impl ToStatement for String {
    fn __to_statement<'a>(
        &'a self,
        client: &'a Client,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<std::borrow::Cow<'a, Statement>, Error>> + 'a>>
    {
        <str as ToStatement>::__to_statement(self.as_str(), client)
    }
}

/// Per-`Client` interner: stable SQL → statement-name mapping. Wasm is
/// single-threaded, so we don't need a Mutex around it (`RefCell` is enough
/// on the `Client` side).
#[derive(Default)]
pub(crate) struct StmtCache {
    next_id: u32,
    by_sql: HashMap<String, String>,
}

impl StmtCache {
    /// Look up or assign a stable name for `sql`. Names look like `pgw_<n>`
    /// — alphanumeric, well under Postgres' 63-char identifier limit.
    pub(crate) fn intern(&mut self, sql: &str) -> String {
        if let Some(name) = self.by_sql.get(sql) {
            return name.clone();
        }
        self.next_id = self.next_id.saturating_add(1);
        let name = format!("pgw_{}", self.next_id);
        self.by_sql.insert(sql.to_string(), name.clone());
        name
    }
}
