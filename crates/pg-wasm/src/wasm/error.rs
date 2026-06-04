//! Error type. Mirrors enough of `tokio_postgres::Error` to be useful for
//! call-site error handling — exposes a SQLSTATE accessor and an
//! `is_closed` check.

use std::error::Error as StdError;
use std::fmt;

use js_sys::Reflect;
use wasm_bindgen::{JsCast, JsValue};

#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
}

#[derive(Debug)]
enum ErrorKind {
    /// Error surfaced by the JS bridge — usually a Postgres ErrorResponse
    /// wrapped in a node-postgres DatabaseError. We try to extract `code`
    /// (SQLSTATE) and `message`.
    Db(DbError),
    /// Connection / I/O failure reported by the JS side (e.g. ECONNRESET).
    Io(String),
    /// `ToSql::to_sql` failed encoding a parameter.
    Encode(Box<dyn StdError + Sync + Send>),
    /// `FromSql::from_sql` failed decoding a column.
    Decode(Box<dyn StdError + Sync + Send>),
    /// `query_one` / `query_opt` got the wrong number of rows.
    RowCount(usize),
    /// Column lookup by name or index failed.
    ColumnNotFound,
}

/// Parsed Postgres error response surfaced through node-postgres.
#[derive(Debug, Default)]
pub struct DbError {
    pub code: Option<String>,
    pub message: String,
    pub detail: Option<String>,
    pub hint: Option<String>,
    pub constraint: Option<String>,
}

impl Error {
    /// SQLSTATE code, if this was a Postgres ErrorResponse.
    #[must_use]
    pub fn code(&self) -> Option<&str> {
        match &self.kind {
            ErrorKind::Db(e) => e.code.as_deref(),
            _ => None,
        }
    }

    /// Structured database error, if any.
    #[must_use]
    pub fn as_db_error(&self) -> Option<&DbError> {
        match &self.kind {
            ErrorKind::Db(e) => Some(e),
            _ => None,
        }
    }

    /// Whether the connection is no longer usable.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        matches!(&self.kind, ErrorKind::Io(_))
    }

    pub(crate) fn from_js(value: JsValue) -> Self {
        let kind = classify_js_error(&value);
        Self { kind }
    }

    pub(crate) fn encode(e: Box<dyn StdError + Sync + Send>) -> Self {
        Self {
            kind: ErrorKind::Encode(e),
        }
    }

    pub(crate) fn decode(e: Box<dyn StdError + Sync + Send>) -> Self {
        Self {
            kind: ErrorKind::Decode(e),
        }
    }

    pub(crate) fn row_count(actual: usize) -> Self {
        Self {
            kind: ErrorKind::RowCount(actual),
        }
    }

    pub(crate) fn column_not_found() -> Self {
        Self {
            kind: ErrorKind::ColumnNotFound,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ErrorKind::Db(e) => match &e.code {
                Some(code) => write!(f, "db error [{code}]: {}", e.message),
                None => write!(f, "db error: {}", e.message),
            },
            ErrorKind::Io(msg) => write!(f, "connection error: {msg}"),
            ErrorKind::Encode(e) => write!(f, "encode error: {e}"),
            ErrorKind::Decode(e) => write!(f, "decode error: {e}"),
            ErrorKind::RowCount(n) => write!(f, "query returned unexpected number of rows: {n}"),
            ErrorKind::ColumnNotFound => write!(f, "column not found"),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match &self.kind {
            ErrorKind::Encode(e) | ErrorKind::Decode(e) => Some(&**e),
            _ => None,
        }
    }
}

/// Inspect a JS error value and classify it into a Db or Io error.
///
/// node-postgres' `DatabaseError` is a plain `Error` with extra string
/// properties like `code`, `severity`, `detail`. Plain connection errors
/// (`ECONNRESET`, `ECONNREFUSED`) come through as Node `Error` objects with
/// a string `code` like `"ECONNRESET"`. We use the presence of `severity`
/// to discriminate the two — only Postgres-level errors carry it.
fn classify_js_error(v: &JsValue) -> ErrorKind {
    if !v.is_object() {
        return ErrorKind::Io(stringify_js(v));
    }

    let severity = get_string_prop(v, "severity");
    if severity.is_some() {
        return ErrorKind::Db(DbError {
            code: get_string_prop(v, "code"),
            message: get_string_prop(v, "message").unwrap_or_default(),
            detail: get_string_prop(v, "detail"),
            hint: get_string_prop(v, "hint"),
            constraint: get_string_prop(v, "constraint"),
        });
    }

    ErrorKind::Io(stringify_js(v))
}

fn get_string_prop(obj: &JsValue, key: &str) -> Option<String> {
    Reflect::get(obj, &JsValue::from_str(key))
        .ok()
        .and_then(|v| v.as_string())
}

fn stringify_js(v: &JsValue) -> String {
    if let Some(s) = v.as_string() {
        return s;
    }
    if let Some(e) = v.dyn_ref::<js_sys::Error>() {
        return format!("{}: {}", String::from(e.name()), String::from(e.message()));
    }
    if let Ok(s) = js_sys::JSON::stringify(v)
        && let Some(s) = s.as_string()
    {
        return s;
    }
    "<opaque JS error>".to_string()
}
