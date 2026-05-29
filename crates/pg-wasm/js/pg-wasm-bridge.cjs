/**
 * JS half of pg-wasm. Owns a node-postgres `pg.Client` and exposes a
 * minimal API to the Rust side via wasm-bindgen.
 *
 * # Why a Submittable?
 *
 * node-postgres' `client.query()` accepts any object with a `submit(connection)`
 * method plus `handleRowDescription`/`handleDataRow`/... handlers. The
 * client routes backend messages to the *active* Submittable and serialises
 * queries through its internal queue. By implementing the Submittable
 * interface we cooperate with that queue — concurrent `queryBinary` calls
 * on the same client serialise correctly instead of stepping on each
 * other's messages — and we don't have to disable the client's own
 * message dispatch.
 *
 * # Binary parameter format
 *
 * pg-protocol's `serialize.bind` auto-marks each value: `null` -> STRING
 * with len -1, `Buffer` -> BINARY, anything else -> STRING. So if we pass
 * `Buffer` values, the per-parameter format codes in the Bind frame come
 * out as binary without us touching the wire format.
 *
 * We pass `binary: true` to bind() to set the result format code to
 * binary too (uniformly across all columns).
 *
 * # Wasm-memory safety
 *
 * `Uint8Array` from wasm-bindgen views wasm linear memory. If wasm memory
 * grows during an `await`, the underlying `ArrayBuffer` detaches. Before
 * any await we copy each `Uint8Array` into a fresh `Buffer.from(u8a)`,
 * which is documented to copy (not view) the data.
 */

"use strict";

// Resolve `pg` from wherever the host project chose to install it.
//
// We're loaded from a wasm-pack `snippets/` subdirectory whose `require`
// resolution context does not include the host's `node_modules`. So we
// try several paths in order:
//
//   1. Standard `require("pg")` — works if a bundler hoisted pg into a
//      reachable parent, or if the snippet ended up under a tree that
//      contains it.
//   2. `require.main.require("pg")` — resolves against the entrypoint's
//      directory, which is usually the host project root.
//   3. Walk up from `process.cwd()` checking `<dir>/node_modules/pg` and
//      a couple of Breez SDK test-specific locations. This covers
//      wasm-pack tests (cwd = `crates/breez-sdk/wasm/`, with pg under
//      `js/postgres-storage/node_modules/`).
//
// If all three fail, surface the standard install hint.
const path = require("path");
const fs = require("fs");

function loadPg() {
  const tried = [];

  try {
    return require("pg");
  } catch (e) {
    tried.push(`require("pg"): ${e.message}`);
  }

  try {
    if (require.main && typeof require.main.require === "function") {
      return require.main.require("pg");
    }
  } catch (e) {
    tried.push(`require.main.require: ${e.message}`);
  }

  let dir = process.cwd();
  for (let depth = 0; depth < 16 && dir && dir !== path.dirname(dir); depth++) {
    const candidates = [
      path.join(dir, "node_modules", "pg"),
      // Breez SDK test layout: pg is installed alongside postgres-storage.
      path.join(dir, "js", "postgres-storage", "node_modules", "pg"),
      path.join(
        dir,
        "crates",
        "breez-sdk",
        "wasm",
        "js",
        "postgres-storage",
        "node_modules",
        "pg"
      ),
    ];
    for (const c of candidates) {
      if (fs.existsSync(c)) {
        try {
          return require(c);
        } catch (e) {
          tried.push(`require(${c}): ${e.message}`);
        }
      }
    }
    dir = path.dirname(dir);
  }

  throw new Error(
    "pg-wasm: 'pg' module not found. Install it in the host project: " +
      "npm install pg@^8.18.0\n" +
      "Attempted resolutions:\n  - " +
      tried.join("\n  - ")
  );
}

const pg = loadPg();
const { Client, Pool } = pg;

/// Subclass of pg.Client that turns on binary-DataRow handling for the
/// underlying connection at construction time. pg.Pool can take a
/// `Client` option that names a constructor to use for each pooled
/// client, so subclassing is the cleanest way to make every pooled
/// client pick up the patch automatically — no need to mutate flags on
/// already-attached connections.
class BinaryClient extends Client {
  constructor(opts) {
    super(opts);
    // Patched `Connection.attachListeners` (see patchPgInternals) reads
    // this flag during `_connect()` to decide whether to install the
    // binary-DataRow Parser.
    this.connection.__pgWasmBinaryDataRow = true;
  }
}

// ── pg-protocol DataRow patch (per-connection) ───────────────────────────────
//
// Background
//
//   pg-protocol's `parseDataRowMessage` decodes every column value through
//   `reader.string(length)` (UTF-8). For binary-format results — which we
//   request via `bind({ binary: true })` so Rust's `FromSql` traits can
//   decode them — arbitrary bytes can't survive the UTF-8 round trip:
//   invalid sequences become U+FFFD and data is lost.
//
// Why not patch globally
//
//   Every `pg.Client` in the process shares `Parser.prototype`. A global
//   patch would change DataRow handling for *all* pg connections,
//   including the existing JS-based postgres-storage and the admin pool
//   used by tests. Those expect string DataRow values (pg's high-level
//   query() ABI). Returning Buffers there breaks them.
//
// What we do instead
//
//   We patch two prototypes, but gate the new behavior on a per-instance
//   flag:
//
//   1. `Parser.prototype.handlePacket`: intercept DataRow (code 0x44)
//      only when `this.__pgWasmBinaryDataRow` is true. Other messages and
//      other parsers go through the original path unchanged.
//
//   2. `Connection.prototype.attachListeners`: when the connection has
//      `__pgWasmBinaryDataRow = true`, construct the Parser ourselves so
//      we can propagate the flag onto it. The original `parse(stream,
//      callback)` function from pg-protocol hides the Parser behind a
//      closure, so we can't reach it after the fact.
//
//   `connectClient` sets `__pgWasmBinaryDataRow = true` on our
//   pg.Client's connection before `await client.connect()`. Other
//   connections in the same process (e.g. test helpers' admin pool,
//   postgres-storage's pool) leave the flag unset and get the vanilla
//   pg-protocol parser behavior.
function findCachedExports(parts) {
  const path = require("path");
  const suffix = path.join(...parts);
  const cacheKey = Object.keys(require.cache).find((k) => k.endsWith(suffix));
  return cacheKey ? require.cache[cacheKey].exports : null;
}

function patchPgInternals() {
  const parserMod = findCachedExports(["pg-protocol", "dist", "parser.js"]);
  const messagesMod = findCachedExports(["pg-protocol", "dist", "messages.js"]);
  const connectionMod = findCachedExports(["pg", "lib", "connection.js"]);
  if (!parserMod || !messagesMod || !connectionMod) {
    throw new Error(
      "pg-wasm: failed to locate pg-protocol parser/messages or pg's " +
        "Connection module. The pg version may be incompatible."
    );
  }
  const { Parser } = parserMod;
  const { DataRowMessage } = messagesMod;
  const Connection = connectionMod;

  if (!Parser.prototype.__pgWasmPatched) {
    const origHandlePacket = Parser.prototype.handlePacket;
    const DATA_ROW_CODE = 0x44; // 'D'
    Parser.prototype.handlePacket = function (offset, code, length, bytes) {
      if (code !== DATA_ROW_CODE || !this.__pgWasmBinaryDataRow) {
        return origHandlePacket.call(this, offset, code, length, bytes);
      }
      // Parse DataRow with bytes() instead of string() so binary-format
      // values round-trip losslessly.
      const reader = this.reader;
      reader.setBuffer(offset, bytes);
      const fieldCount = reader.int16();
      const fields = new Array(fieldCount);
      for (let i = 0; i < fieldCount; i++) {
        const len = reader.int32();
        if (len === -1) {
          fields[i] = null;
        } else {
          // bytes() returns a Buffer slice that shares its ArrayBuffer
          // with the parser's working buffer. Copy it so it survives the
          // next packet.
          fields[i] = Buffer.from(reader.bytes(len));
        }
      }
      return new DataRowMessage(length, fields);
    };
    Parser.prototype.__pgWasmPatched = true;
  }

  if (!Connection.prototype.__pgWasmPatched) {
    const origAttachListeners = Connection.prototype.attachListeners;
    Connection.prototype.attachListeners = function (stream) {
      if (!this.__pgWasmBinaryDataRow) {
        return origAttachListeners.call(this, stream);
      }
      // Inline-equivalent of the original attachListeners, but with a
      // locally-constructed Parser whose `__pgWasmBinaryDataRow` we
      // propagate from the connection.
      const parser = new Parser();
      parser.__pgWasmBinaryDataRow = true;
      const self = this;
      stream.on("data", (buffer) =>
        parser.parse(buffer, (msg) => {
          const eventName = msg.name === "error" ? "errorMessage" : msg.name;
          if (self._emitMessage) self.emit("message", msg);
          self.emit(eventName, msg);
        })
      );
    };
    Connection.prototype.__pgWasmPatched = true;
  }
}

/**
 * Submittable that prepares a statement and fetches the
 * server-inferred parameter type OIDs plus the resulting RowDescription
 * (if any).
 *
 * Sequence: Parse → Describe Statement → Sync. The backend replies with
 * ParseComplete, ParameterDescription, then either RowDescription or
 * NoData, then ReadyForQuery. We resolve with `{ paramOids, fieldOids,
 * fieldNames }` so the Rust side can build a `Statement` whose
 * `params()` match what `ToSql::accepts` expects.
 *
 * pg's Client doesn't route ParameterDescription to active queries
 * (none of its built-in Query impls need it), so we attach our own
 * listener on the connection for the duration of this Submittable and
 * remove it on completion.
 */
class BinaryPrepare {
  constructor(text, name, resolve, reject) {
    this.text = text;
    this.name = name;
    this._resolve = resolve;
    this._reject = reject;
    this._paramOids = null;
    this._desc = null;
    this._error = null;
    this._connection = null;
    this._onParameterDescription = (msg) => {
      // msg.dataTypeIDs is the wire-order array of inferred parameter
      // type OIDs.
      this._paramOids = msg.dataTypeIDs;
    };
  }

  submit(connection) {
    this._connection = connection;
    // pg doesn't route ParameterDescription anywhere by default; we
    // listen directly on the connection for the duration of this query
    // and detach in cleanup.
    connection.on("parameterDescription", this._onParameterDescription);
    try {
      const cachedText = this.name
        ? connection.parsedStatements[this.name]
        : undefined;
      if (cachedText !== undefined && cachedText !== this.text) {
        // Hash collision with a previously-parsed statement on this
        // connection (extremely rare). Surface it loudly rather than
        // describing the wrong SQL.
        throw new Error(
          `pg-wasm: prepared-statement name collision on '${this.name}'. ` +
            `Cached SQL differs from requested SQL.`
        );
      }
      if (!cachedText) {
        connection.parse({
          name: this.name,
          text: this.text,
          types: [],
        });
      }
      // Describe Statement → ParameterDescription + RowDescription/NoData.
      connection.describe({ type: "S", name: this.name });
      connection.sync();
      return null;
    } catch (err) {
      this._cleanup();
      this._reject(err);
      return err;
    }
  }

  _cleanup() {
    if (this._connection) {
      this._connection.off(
        "parameterDescription",
        this._onParameterDescription
      );
      this._connection = null;
    }
  }

  handleRowDescription(msg) {
    this._desc = msg;
  }

  handleDataRow() {
    // unreachable — Describe Statement doesn't produce rows
  }

  handleCommandComplete() {
    // unreachable — Describe Statement doesn't produce a CommandComplete
  }

  handleEmptyQuery() {
    // unreachable
  }

  handleNoData() {
    // statement has no result columns; _desc stays null
  }

  handleError(err) {
    this._error = err;
  }

  handleReadyForQuery() {
    this._cleanup();
    if (this._error) {
      this._reject(this._error);
      return;
    }
    const paramOids = new Uint32Array(this._paramOids || []);
    const fieldOids = this._desc
      ? new Uint32Array(this._desc.fields.map((f) => f.dataTypeID))
      : new Uint32Array();
    const fieldNames = this._desc ? this._desc.fields.map((f) => f.name) : [];
    this._resolve({ paramOids, fieldOids, fieldNames });
  }
}

/**
 * Submittable that runs Parse + Bind + Describe + Execute + Sync with
 * binary parameter and result formats.
 *
 * Object shape required by node-postgres' `Client._activeQuery` routing:
 * `submit`, `handleRowDescription`, `handleDataRow`,
 * `handleCommandComplete`, `handleEmptyQuery`, `handleError`,
 * `handleReadyForQuery`. We only need the extended-protocol handlers;
 * portal-suspended / copy-in / copy-data aren't reachable from
 * `queryBinary` so we omit them.
 */
class BinaryQuery {
  constructor(text, name, paramOids, paramBuffers, resolve, reject) {
    this.text = text;
    this.name = name;
    this._paramOids = Array.from(paramOids);
    this._paramBuffers = paramBuffers;
    this._resolve = resolve;
    this._reject = reject;
    this._desc = null;
    this._rows = [];
    this._tag = null;
    this._error = null;
  }

  submit(connection) {
    try {
      // pg-protocol caches parsed statement names in
      // `connection.parsedStatements[name]`. Skip the Parse round-trip
      // if we've already sent it under the same name AND the cached SQL
      // matches. Unnamed statements (`name === ''`) always re-parse.
      //
      // Defensive `cachedText === this.text` check: prepared-statement
      // names are stable hashes of the SQL; the equality should always
      // hold and a mismatch indicates a hash collision that we'd rather
      // surface loudly than silently describe the wrong statement.
      const cachedText = this.name
        ? connection.parsedStatements[this.name]
        : undefined;
      if (cachedText !== undefined && cachedText !== this.text) {
        throw new Error(
          `pg-wasm: prepared-statement name collision on '${this.name}'. ` +
            `Cached SQL differs from requested SQL.`
        );
      }
      if (!cachedText) {
        connection.parse({
          name: this.name,
          text: this.text,
          types: this._paramOids,
        });
      }
      connection.bind({
        portal: "",
        statement: this.name,
        values: this._paramBuffers,
        binary: true, // request binary result format
      });
      // Describe Portal — Postgres only emits a RowDescription in response
      // to Describe; without this we'd see DataRow values without any
      // column metadata, leaving the Rust side with an empty `columns`
      // vec and `Row::get` failing with "column not found".
      //
      // pg.Query.prepare() does the same thing for the same reason; see
      // node-postgres/packages/pg/lib/query.js -> Query.prototype.prepare.
      connection.describe({ type: "P", name: "" });
      connection.execute({ portal: "" });
      connection.sync();
      return null;
    } catch (err) {
      // pg.Client treats a non-null return from submit() as an immediate
      // submit-time failure and won't expect handlers to fire.
      this._reject(err);
      return err;
    }
  }

  handleRowDescription(msg) {
    this._desc = msg;
  }

  handleDataRow(msg) {
    // msg.fields is (Buffer | null)[] — binary-format column values,
    // thanks to the Parser.handlePacket DataRow patch above.
    this._rows.push(msg.fields);
  }

  handleCommandComplete(msg /* { text, ... } */) {
    this._tag = msg.text;
  }

  handleEmptyQuery() {
    // No-op. We'll still get ReadyForQuery and resolve with empty rows.
  }

  handleError(err) {
    // Hold the error until ReadyForQuery so we resolve/reject exactly once
    // at the end of the round-trip. node-postgres has already cleared
    // _activeQuery before invoking us.
    this._error = err;
  }

  handleReadyForQuery() {
    if (this._error) {
      this._reject(this._error);
      return;
    }
    this._resolve(new QueryResult(this._desc, this._rows, this._tag));
  }
}

/**
 * Plain-data result handed back across the wasm boundary. Getters match
 * the wasm-bindgen extern declarations in `src/wasm/js.rs`.
 */
class QueryResult {
  constructor(desc, rawRows, tag) {
    this._desc = desc;
    this._rawRows = rawRows;
    this._tag = tag;
  }

  get fieldOids() {
    // Return a Uint32Array so wasm-bindgen unmarshals it into `Vec<u32>`
    // on the Rust side. A plain JS Array would come across as an empty
    // Vec because the ABI for `Vec<u32>` is the memory-pointer form
    // (typed array), not a heterogeneous JS Array.
    if (!this._desc) return new Uint32Array();
    const fields = this._desc.fields;
    const out = new Uint32Array(fields.length);
    for (let i = 0; i < fields.length; i++) out[i] = fields[i].dataTypeID;
    return out;
  }

  get fieldNames() {
    // Plain JS Array is correct here — the Rust side reads it as
    // `js_sys::Array` and pulls strings via `.get(i).as_string()`.
    return this._desc ? this._desc.fields.map((f) => f.name) : [];
  }

  get rows() {
    // Map (Buffer | null)[][] -> (Uint8Array | null)[][] for wasm-bindgen.
    //
    // We deliberately pass `buf` (a Node `Buffer`, which extends
    // `Uint8Array`) to `new Uint8Array(typedArray)`. That form *copies*
    // the bytes into a freshly-allocated standalone Uint8Array.
    //
    // The simpler `new Uint8Array(buf.buffer, buf.byteOffset,
    // buf.byteLength)` form would create a *view* into Node's shared
    // Buffer pool. Such views appear empty (`u8a.length() == 0`) once
    // they cross the wasm-bindgen boundary — js_sys::Uint8Array doesn't
    // track non-zero byteOffsets into a larger ArrayBuffer here.
    // Copying sidesteps the issue.
    return this._rawRows.map((row) =>
      row.map((buf) => (buf == null ? null : new Uint8Array(buf)))
    );
  }

  get rowsAffected() {
    return parseAffected(this._tag);
  }
}

/**
 * Parse the CommandComplete tag for an affected-row count.
 *
 * Examples:
 *   "INSERT 0 5" -> 5      (oid, count)
 *   "UPDATE 3"   -> 3
 *   "DELETE 2"   -> 2
 *   "SELECT 12"  -> 12
 *   "BEGIN"      -> null   (no count)
 */
function parseAffected(tag) {
  if (!tag) return null;
  const parts = tag.split(" ");
  const last = parts[parts.length - 1];
  const n = Number(last);
  return Number.isFinite(n) && Number.isInteger(n) ? n : null;
}

/**
 * Connect a fresh pg.Client. The Rust side gets a `JsClient` wrapper.
 */
async function connectClient(connectionString) {
  // Patches are gated on per-instance flags, so they're safe to apply
  // once globally — they only affect connections opted in via
  // `__pgWasmBinaryDataRow`.
  patchPgInternals();
  const client = new BinaryClient({ connectionString });
  await client.connect();
  return new JsClient(client, /* pooled */ false);
}

function createPool(connectionString) {
  patchPgInternals();
  // pg.Pool's `Client` option specifies the constructor to use for each
  // pooled connection. Using our BinaryClient subclass makes every
  // checkout flip the binary-DataRow flag on its connection
  // automatically.
  const pool = new Pool({ connectionString, Client: BinaryClient });
  return new JsPool(pool);
}

class JsPool {
  constructor(pgPool) {
    this._pool = pgPool;
  }

  async connect() {
    const client = await this._pool.connect();
    return new JsClient(client, /* pooled */ true);
  }

  end() {
    try {
      const p = this._pool.end();
      if (p && typeof p.catch === "function") p.catch(() => {});
    } catch (_err) {
      // ignore
    }
  }
}

class JsClient {
  constructor(client, pooled) {
    this._client = client;
    this._pooled = pooled;
  }

  prepareStatement(text, statementName) {
    return new Promise((resolve, reject) => {
      const sub = new BinaryPrepare(text, statementName, resolve, reject);
      this._client.query(sub);
    });
  }

  queryBinary(text, statementName, paramOids, paramValues) {
    // Snapshot the wasm Uint8Arrays into Node Buffers *now*, before any
    // await. `Buffer.from(u8a)` copies the bytes; the returned Buffer
    // survives subsequent wasm memory growth.
    const paramBuffers = paramValues.map((v) =>
      v == null ? null : Buffer.from(v)
    );

    return new Promise((resolve, reject) => {
      const submittable = new BinaryQuery(
        text,
        statementName,
        paramOids,
        paramBuffers,
        resolve,
        reject
      );
      // client.query(submittable) hands us to pg's queue.
      this._client.query(submittable);
    });
  }

  async simpleQuery(sql) {
    // pg's high-level query() handles the simple query protocol for us.
    // Used for BEGIN/COMMIT/ROLLBACK and ad-hoc DDL.
    await this._client.query(sql);
  }

  /**
   * Best-effort cleanup. For a pooled client this releases the
   * connection back to its pool; for a standalone client it closes the
   * TCP connection. Fire-and-forget so it's safe to call from Rust's
   * Drop.
   */
  close() {
    try {
      if (this._pooled) {
        this._client.release();
      } else {
        const p = this._client.end();
        if (p && typeof p.catch === "function") p.catch(() => {});
      }
    } catch (_err) {
      // ignore
    }
  }
}

module.exports = { connectClient, createPool };
