//! Error types for the `spark-postgres` crate.

/// Errors that can occur when working with `PostgreSQL` storage.
#[derive(Debug, thiserror::Error)]
pub enum PostgresError {
    /// Connection-related errors (pool exhaustion, timeouts, connection refused).
    /// These are often transient and may be retried.
    #[error("Connection error: {0}")]
    Connection(String),

    /// Database initialization error (invalid connection string, TLS config failures, migration failures).
    #[error("Initialization error: {0}")]
    Initialization(String),

    /// General database errors (query failures, constraint violations, etc.).
    #[error("Database error: {0}")]
    Database(String),
}
