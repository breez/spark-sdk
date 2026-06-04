//! Prepared statement + per-client cache.
//!
//! Mirrors `tokio_postgres::Statement`. The actual `Parse` frame is sent
//! lazily on first execution; this struct just records the SQL text,
//! the parameter type OIDs, and a stable name for pg-protocol's
//! `parsedStatements` cache to key on.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::future::Future;
use std::hash::{Hash, Hasher};

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

/// Per-`Client` cache: stable SQL → `(statement-name, prepared
/// Statement)`. Wasm is single-threaded so a `RefCell` around it is
/// enough on the `Client` side.
///
/// The `name` part is interned eagerly (so `prepare_typed` and the
/// describe-round-trip in `prepare` agree on the name); the `Statement`
/// half is populated only by `prepare` after it has fetched
/// server-inferred parameter types. `prepare_typed` leaves the
/// `Statement` slot empty because it doesn't round-trip and doesn't
/// know whether the caller's types are authoritative.
#[derive(Default)]
pub(crate) struct StmtCache {
    next_id: u32,
    entries: HashMap<String, CacheEntry>,
}

struct CacheEntry {
    name: String,
    prepared: Option<Statement>,
}

impl StmtCache {
    /// Look up or assign a stable name for `sql`.
    ///
    /// The name is a stable hash of the SQL text — `pgw_<hex>` — so the
    /// same SQL maps to the same name regardless of which Rust `Client`
    /// wrapper is asking. This matters because `pg.Pool` recycles its
    /// underlying `pg.Client` instances: a fresh Rust `Client` from
    /// `pool.get()` may sit on top of a JS connection whose
    /// `connection.parsedStatements` cache already contains entries
    /// from a previous checkout. Counter-based names would collide
    /// (different SQL, same name) and `Describe Statement` would return
    /// parameter info for the *previously* cached statement.
    ///
    /// `DefaultHasher` is SipHash-1-3; collisions for the ~100 SQL
    /// strings the SDK uses are astronomically improbable. The JS
    /// bridge does a defensive `parsedStatements[name] === text` check
    /// and falls back gracefully if one ever happens.
    pub(crate) fn intern(&mut self, sql: &str) -> String {
        if let Some(e) = self.entries.get(sql) {
            return e.name.clone();
        }
        // Bump the counter so generated names stay distinct in any
        // legacy code paths that still read it; unused for the hash.
        self.next_id = self.next_id.saturating_add(1);
        let mut h = DefaultHasher::new();
        sql.hash(&mut h);
        let name = format!("pgw_{:016x}", h.finish());
        self.entries.insert(
            sql.to_string(),
            CacheEntry {
                name: name.clone(),
                prepared: None,
            },
        );
        name
    }

    /// Look up an already-prepared statement by SQL text, if any.
    pub(crate) fn get(&self, sql: &str) -> Option<Statement> {
        self.entries.get(sql).and_then(|e| e.prepared.clone())
    }

    /// Store a fully-prepared statement (with server-inferred types) in
    /// the cache. `intern` is expected to have run already.
    pub(crate) fn store(&mut self, sql: &str, stmt: Statement) {
        if let Some(e) = self.entries.get_mut(sql) {
            e.prepared = Some(stmt);
        } else {
            // Shouldn't happen in practice — prepare always interns
            // first — but be defensive.
            self.entries.insert(
                sql.to_string(),
                CacheEntry {
                    name: stmt.name().to_string(),
                    prepared: Some(stmt),
                },
            );
        }
    }
}
