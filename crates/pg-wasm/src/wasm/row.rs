//! `Row` + `Column` + `RowIndex` trait.
//!
//! Each `Row` holds an `Arc<[Column]>` (shared across all rows in a
//! result set) and a `Vec<Option<Bytes>>` of binary-encoded values.
//! Decoding is deferred to `try_get` / `get`, which call
//! `FromSql::from_sql` from `postgres-types`.

use std::sync::Arc;

use bytes::Bytes;
use postgres_types::{FromSql, Type, WrongType};

use super::error::Error;

/// Column metadata: name + Postgres type.
#[derive(Debug)]
pub struct Column {
    pub(crate) name: String,
    pub(crate) ty: Type,
}

impl Column {
    /// Column name as returned by the backend (from `RowDescription`).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Postgres type the value is binary-encoded in.
    #[must_use]
    pub fn type_(&self) -> &Type {
        &self.ty
    }
}

/// One row of a query result. Columns are decoded on demand via `get` /
/// `try_get`, mirroring `tokio_postgres::Row`.
pub struct Row {
    columns: Arc<[Column]>,
    values: Vec<Option<Bytes>>,
}

// Single-threaded wasm; backing data (`Arc<[Column]>`, `Vec<Bytes>`) is
// already plain Rust. Matches the `Send + Sync` expectation of code
// written against `tokio_postgres::Row`.
unsafe impl Send for Row {}
unsafe impl Sync for Row {}

impl Row {
    pub(crate) fn new(columns: Arc<[Column]>, values: Vec<Option<Bytes>>) -> Self {
        Self { columns, values }
    }

    /// Column descriptors, in order.
    #[must_use]
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// Number of columns.
    #[must_use]
    pub fn len(&self) -> usize {
        self.columns.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    /// Decode the value at `idx` into `T`. Panics on type mismatch, NULL
    /// for a non-`Option` `T`, or missing column. Use [`try_get`](Self::try_get)
    /// to surface the error instead.
    pub fn get<'a, I, T>(&'a self, idx: I) -> T
    where
        I: RowIndex + std::fmt::Display + Clone,
        T: FromSql<'a>,
    {
        let display = idx.clone();
        match self.try_get_impl(&idx) {
            Ok(v) => v,
            Err(e) => panic!("error retrieving column {display}: {e}"),
        }
    }

    /// Decode the value at `idx` into `T`, returning any error.
    pub fn try_get<'a, I, T>(&'a self, idx: I) -> Result<T, Error>
    where
        I: RowIndex,
        T: FromSql<'a>,
    {
        self.try_get_impl(&idx)
    }

    fn try_get_impl<'a, I, T>(&'a self, idx: &I) -> Result<T, Error>
    where
        I: RowIndex,
        T: FromSql<'a>,
    {
        let i = idx
            .__idx(&self.columns)
            .ok_or_else(Error::column_not_found)?;
        let col = &self.columns[i];

        if !T::accepts(&col.ty) {
            return Err(Error::decode(Box::new(WrongType::new::<T>(col.ty.clone()))));
        }

        match self.values[i].as_deref() {
            Some(bytes) => T::from_sql(&col.ty, bytes).map_err(Error::decode),
            None => T::from_sql_null(&col.ty).map_err(Error::decode),
        }
    }
}

/// Sealed trait identifying a column by index or name. Mirrors
/// `tokio_postgres::row::RowIndex` — `usize` for positional, `&str` /
/// `String` for named.
pub trait RowIndex {
    #[doc(hidden)]
    fn __idx(&self, columns: &[Column]) -> Option<usize>;
}

impl RowIndex for usize {
    fn __idx(&self, columns: &[Column]) -> Option<usize> {
        if *self < columns.len() {
            Some(*self)
        } else {
            None
        }
    }
}

impl RowIndex for str {
    fn __idx(&self, columns: &[Column]) -> Option<usize> {
        columns.iter().position(|c| c.name == self)
    }
}

impl<T> RowIndex for &T
where
    T: ?Sized + RowIndex,
{
    fn __idx(&self, columns: &[Column]) -> Option<usize> {
        T::__idx(*self, columns)
    }
}

impl RowIndex for String {
    fn __idx(&self, columns: &[Column]) -> Option<usize> {
        str::__idx(self.as_str(), columns)
    }
}
