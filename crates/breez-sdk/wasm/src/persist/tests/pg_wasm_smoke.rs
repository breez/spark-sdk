//! Smoke test for the `pg-wasm` crate.
//!
//! Runs against the shared testcontainers-managed Postgres exposed by
//! `js/postgres-test-helpers.cjs`. The goal is to validate the *plumbing*
//! end-to-end: open a connection through pg-wasm, send a prepared
//! statement with a binary parameter, decode a binary result, and verify
//! it round-trips.
//!
//! This test will be the first thing to break if the JS bridge contract
//! drifts. Specifically it exercises:
//!
//!   * `Config::connect` opening a `pg.Client` on the JS side,
//!   * `Client::prepare_typed` interning a statement name,
//!   * `ToSql` binary encoding of an `i64` parameter,
//!   * pg-protocol's auto-detection of `Buffer` params as binary format,
//!   * `bind({ binary: true })` requesting binary result format,
//!   * `FromSql` binary decoding back to `i64`.
//!
//! If this passes, every other supported type just needs the matching
//! `ToSql`/`FromSql` impl; the wire path is the same.

use pg_wasm::types::Type;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

#[wasm_bindgen(module = "js/postgres-test-helpers.cjs")]
extern "C" {
    #[wasm_bindgen(js_name = "createTestConnectionString", catch)]
    async fn create_test_connection_string(test_name: &str) -> Result<JsValue, JsValue>;
}

#[wasm_bindgen_test]
async fn smoke_select_int8_roundtrip() {
    let conn_str = create_test_connection_string("pg_wasm_smoke")
        .await
        .expect("create test connection string")
        .as_string()
        .expect("connection string is a string");

    let client = pg_wasm::connect(&conn_str)
        .await
        .expect("pg_wasm::connect");

    let stmt = client
        .prepare_typed("SELECT $1::int8", &[Type::INT8])
        .await
        .expect("prepare_typed");

    let value: i64 = 0x0123_4567_89AB_CDEF;
    let row = client
        .query_one(&stmt, &[&value])
        .await
        .expect("query_one");

    let got: i64 = row.get(0);
    assert_eq!(got, value, "binary i64 round-trip must preserve all bytes");
}

#[wasm_bindgen_test]
async fn smoke_text_param_and_result() {
    let conn_str = create_test_connection_string("pg_wasm_smoke")
        .await
        .expect("create test connection string")
        .as_string()
        .expect("connection string is a string");

    let client = pg_wasm::connect(&conn_str).await.expect("connect");
    let stmt = client
        .prepare_typed("SELECT $1::text", &[Type::TEXT])
        .await
        .expect("prepare_typed");

    let payload = "héllo, π — 🦀";
    let row = client
        .query_one(&stmt, &[&payload])
        .await
        .expect("query_one");

    let got: String = row.get(0);
    assert_eq!(got, payload);
}

#[wasm_bindgen_test]
async fn smoke_null_param_and_result() {
    let conn_str = create_test_connection_string("pg_wasm_smoke")
        .await
        .expect("create test connection string")
        .as_string()
        .expect("connection string is a string");

    let client = pg_wasm::connect(&conn_str).await.expect("connect");
    let stmt = client
        .prepare_typed("SELECT $1::int8", &[Type::INT8])
        .await
        .expect("prepare_typed");

    let null: Option<i64> = None;
    let row = client.query_one(&stmt, &[&null]).await.expect("query_one");

    let got: Option<i64> = row.get(0);
    assert!(got.is_none(), "NULL must round-trip as Option::None");
}

#[wasm_bindgen_test]
async fn smoke_query_with_str_statement() {
    // Validates the `ToStatement` impl for `&str`, including the
    // parameterised path which depends on `Client::prepare` doing a
    // server-side Describe Statement round-trip to fetch inferred
    // parameter type OIDs.
    let conn_str = create_test_connection_string("pg_wasm_smoke_str_stmt")
        .await
        .expect("create test connection string")
        .as_string()
        .expect("connection string is a string");

    let client = pg_wasm::connect(&conn_str).await.expect("connect");

    // Parameterless &str.
    let affected = client
        .execute("CREATE TABLE t (id BIGINT)", &[])
        .await
        .expect("execute with &str");
    assert_eq!(affected, 0); // DDL doesn't report rows affected.

    let row = client
        .query_one("SELECT 42::int8", &[])
        .await
        .expect("query_one with &str");
    let v: i64 = row.get(0);
    assert_eq!(v, 42);

    // Parameterised &str — relies on server-inferred OIDs.
    let row = client
        .query_one("SELECT $1::int8 + 100", &[&7_i64])
        .await
        .expect("query_one with parameterised &str");
    let v: i64 = row.get(0);
    assert_eq!(v, 107);

    // Second call exercises the prepare-cache: should not round-trip
    // Describe Statement again.
    let row = client
        .query_one("SELECT $1::int8 + 100", &[&8_i64])
        .await
        .expect("cached parameterised &str");
    let v: i64 = row.get(0);
    assert_eq!(v, 108);
}

#[wasm_bindgen_test]
async fn smoke_pool_roundtrip() {
    let conn_str = create_test_connection_string("pg_wasm_smoke_pool")
        .await
        .expect("create test connection string")
        .as_string()
        .expect("connection string is a string");

    let pool = pg_wasm::Pool::new(&conn_str).expect("create pool");

    // Two sequential checkouts validate that release() returns the
    // client to the pool cleanly (not end()ing the underlying TCP
    // connection), and that the binary-DataRow patch is wired up via
    // the BinaryClient subclass on pool-instantiated clients.
    for expected in [1_i64, 2_i64] {
        let client = pool.get().await.expect("pool.get");
        let stmt = client
            .prepare_typed("SELECT $1::int8", &[Type::INT8])
            .await
            .expect("prepare");
        let row = client.query_one(&stmt, &[&expected]).await.expect("query");
        let got: i64 = row.get(0);
        assert_eq!(got, expected);
        // Drop happens at end of loop iteration; releases the client.
    }

    pool.close();
}

#[wasm_bindgen_test]
async fn smoke_transaction_commit_persists() {
    let conn_str = create_test_connection_string("pg_wasm_smoke_tx_commit")
        .await
        .expect("create test connection string")
        .as_string()
        .expect("connection string is a string");

    let mut client = pg_wasm::connect(&conn_str).await.expect("connect");
    client
        .batch_execute("CREATE TABLE t (id BIGINT PRIMARY KEY)")
        .await
        .expect("create table");

    let tx = client.transaction().await.expect("BEGIN");
    let insert = tx
        .prepare_typed("INSERT INTO t (id) VALUES ($1)", &[Type::INT8])
        .await
        .expect("prepare");
    tx.execute(&insert, &[&7_i64]).await.expect("insert");
    tx.commit().await.expect("COMMIT");

    let count_stmt = client
        .prepare_typed("SELECT count(*)::int8 FROM t", &[])
        .await
        .expect("prepare count");
    let row = client
        .query_one(&count_stmt, &[])
        .await
        .expect("query count");
    let count: i64 = row.get(0);
    assert_eq!(count, 1, "committed insert must persist");
}

#[wasm_bindgen_test]
async fn smoke_transaction_rollback_discards() {
    let conn_str = create_test_connection_string("pg_wasm_smoke_tx_rollback")
        .await
        .expect("create test connection string")
        .as_string()
        .expect("connection string is a string");

    let mut client = pg_wasm::connect(&conn_str).await.expect("connect");
    client
        .batch_execute("CREATE TABLE t (id BIGINT PRIMARY KEY)")
        .await
        .expect("create table");

    let tx = client.transaction().await.expect("BEGIN");
    let insert = tx
        .prepare_typed("INSERT INTO t (id) VALUES ($1)", &[Type::INT8])
        .await
        .expect("prepare");
    tx.execute(&insert, &[&7_i64]).await.expect("insert");
    tx.rollback().await.expect("ROLLBACK");

    let count_stmt = client
        .prepare_typed("SELECT count(*)::int8 FROM t", &[])
        .await
        .expect("prepare count");
    let row = client
        .query_one(&count_stmt, &[])
        .await
        .expect("query count");
    let count: i64 = row.get(0);
    assert_eq!(count, 0, "rolled-back insert must not persist");
}

#[wasm_bindgen_test]
async fn smoke_execute_returns_rows_affected() {
    let conn_str = create_test_connection_string("pg_wasm_smoke_exec")
        .await
        .expect("create test connection string")
        .as_string()
        .expect("connection string is a string");

    let client = pg_wasm::connect(&conn_str).await.expect("connect");

    client
        .batch_execute("CREATE TABLE t (id BIGINT NOT NULL)")
        .await
        .expect("create table");

    let insert = client
        .prepare_typed(
            "INSERT INTO t (id) VALUES ($1), ($2), ($3)",
            &[Type::INT8, Type::INT8, Type::INT8],
        )
        .await
        .expect("prepare insert");

    let affected = client
        .execute(&insert, &[&1_i64, &2_i64, &3_i64])
        .await
        .expect("execute insert");

    assert_eq!(affected, 3, "INSERT must report 3 rows affected");
}
