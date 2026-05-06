/**
 * CommonJS implementation for Node.js MySQL Token Store.
 *
 * Mirrors `postgres-token-store/index.cjs` for MySQL 8.0+. See
 * `mysql-storage/index.cjs` for SQL translation rules.
 */

let mysql;
try {
  const mainModule = require.main;
  if (mainModule) {
    mysql = mainModule.require("mysql2/promise");
  } else {
    mysql = require("mysql2/promise");
  }
} catch (error) {
  try {
    mysql = require("mysql2/promise");
  } catch (fallbackError) {
    throw new Error(
      `mysql2 not found. Please install it in your project: npm install mysql2@^3.11.0\n` +
        `Original error: ${error.message}\nFallback error: ${fallbackError.message}`
    );
  }
}

const { TokenStoreError } = require("./errors.cjs");
const { MysqlTokenStoreMigrationManager } = require("./migrations.cjs");

const TOKEN_STORE_WRITE_LOCK_NAME = "token_store_write_lock";
const WRITE_LOCK_TIMEOUT_SECS = 30;

const SPENT_MARKER_CLEANUP_THRESHOLD_MS = 5 * 60 * 1000;
const RESERVATION_TIMEOUT_SECS = 300;

function parseJson(value) {
  if (value == null) return null;
  if (typeof value === "string") return JSON.parse(value);
  return value;
}

function toBool(value) {
  if (value == null) return null;
  if (typeof value === "boolean") return value;
  return value === 1 || value === "1" || value === true;
}

function buildPlaceholders(n) {
  return new Array(n).fill("?").join(", ");
}

class MysqlTokenStore {
  constructor(pool, logger = null) {
    this.pool = pool;
    this.logger = logger;
  }

  async initialize() {
    try {
      const migrationManager = new MysqlTokenStoreMigrationManager(this.logger);
      await migrationManager.migrate(this.pool);
      return this;
    } catch (error) {
      throw new TokenStoreError(
        `Failed to initialize MySQL token store: ${error.message}`,
        error
      );
    }
  }

  async close() {
    if (this.pool) {
      await this.pool.end();
      this.pool = null;
    }
  }

  /**
   * Run a function inside a transaction holding the named write lock. Reserved
   * for operations whose correctness depends on serializing the
   * available-output set (`reserveTokenOutputs`, `setTokensOutputs`).
   * @param {function(import('mysql2/promise').PoolConnection): Promise<T>} fn
   * @returns {Promise<T>}
   * @template T
   */
  async _withWriteTransaction(fn) {
    const conn = await this.pool.getConnection();
    let lockAcquired = false;
    try {
      const [lockRows] = await conn.query(
        "SELECT GET_LOCK(?, ?) AS acquired",
        [TOKEN_STORE_WRITE_LOCK_NAME, WRITE_LOCK_TIMEOUT_SECS]
      );
      if (!lockRows || lockRows[0].acquired !== 1) {
        throw new TokenStoreError(
          `Failed to acquire token store write lock within ${WRITE_LOCK_TIMEOUT_SECS}s`
        );
      }
      lockAcquired = true;

      await conn.beginTransaction();
      const result = await fn(conn);
      await conn.commit();
      return result;
    } catch (error) {
      await conn.rollback().catch(() => {});
      throw error;
    } finally {
      if (lockAcquired) {
        await conn
          .query("SELECT RELEASE_LOCK(?)", [TOKEN_STORE_WRITE_LOCK_NAME])
          .catch(() => {});
      }
      conn.release();
    }
  }

  /**
   * Run a function inside a transaction without the advisory lock. Used by
   * operations scoped to a single reservation_id (`cancelReservation`)
   * where row-level FK + InnoDB MVCC suffice and the global lock would only
   * add contention.
   * @param {function(import('mysql2/promise').PoolConnection): Promise<T>} fn
   * @returns {Promise<T>}
   * @template T
   */
  async _withTransaction(fn) {
    const conn = await this.pool.getConnection();
    try {
      await conn.beginTransaction();
      const result = await fn(conn);
      await conn.commit();
      return result;
    } catch (error) {
      await conn.rollback().catch(() => {});
      throw error;
    } finally {
      conn.release();
    }
  }

  // ===== TokenOutputStore Methods =====

  async setTokensOutputs(tokenOutputs, refreshStartedAtMs) {
    try {
      const refreshTimestamp = new Date(refreshStartedAtMs);

      await this._withWriteTransaction(async (conn) => {
        await this._cleanupStaleReservations(conn);

        const [swapRows] = await conn.query(
          `SELECT
            (SELECT EXISTS(SELECT 1 FROM token_reservations WHERE purpose = 'Swap')) AS has_active_swap,
            COALESCE(
              (SELECT (last_completed_at >= ?) FROM token_swap_status WHERE id = 1),
              0
            ) AS swap_completed`,
          [refreshTimestamp]
        );
        const hasActiveSwap = !!swapRows[0].has_active_swap;
        const swapCompleted = !!swapRows[0].swap_completed;
        if (hasActiveSwap || swapCompleted) {
          return;
        }

        const cleanupCutoff = new Date(
          refreshTimestamp.getTime() - SPENT_MARKER_CLEANUP_THRESHOLD_MS
        );
        await conn.query(
          "DELETE FROM token_spent_outputs WHERE spent_at < ?",
          [cleanupCutoff]
        );

        const [spentRows] = await conn.query(
          "SELECT output_id FROM token_spent_outputs WHERE spent_at >= ?",
          [refreshTimestamp]
        );
        const spentIds = new Set(spentRows.map((r) => r.output_id));

        await conn.query(
          "DELETE FROM token_outputs WHERE reservation_id IS NULL AND added_at < ?",
          [refreshTimestamp]
        );

        const incomingOutputIds = new Set();
        for (const to of tokenOutputs) {
          for (const o of to.outputs) {
            incomingOutputIds.add(o.output.id);
          }
        }

        const [reservedRows] = await conn.query(
          `SELECT r.id, o.id AS output_id
           FROM token_reservations r
           JOIN token_outputs o ON o.reservation_id = r.id`
        );

        const reservationOutputs = new Map();
        for (const row of reservedRows) {
          if (!reservationOutputs.has(row.id)) {
            reservationOutputs.set(row.id, []);
          }
          reservationOutputs.get(row.id).push(row.output_id);
        }

        const reservationsToDelete = [];
        const outputsToRemoveFromReservation = [];
        for (const [reservationId, outputIds] of reservationOutputs) {
          const validIds = outputIds.filter((id) => incomingOutputIds.has(id));
          if (validIds.length === 0) {
            reservationsToDelete.push(reservationId);
          } else {
            for (const id of outputIds) {
              if (!incomingOutputIds.has(id)) {
                outputsToRemoveFromReservation.push(id);
              }
            }
          }
        }

        if (reservationsToDelete.length > 0) {
          const placeholders = buildPlaceholders(reservationsToDelete.length);
          await conn.query(
            `DELETE FROM token_outputs WHERE reservation_id IN (${placeholders})`,
            reservationsToDelete
          );
          await conn.query(
            `DELETE FROM token_reservations WHERE id IN (${placeholders})`,
            reservationsToDelete
          );
        }

        if (outputsToRemoveFromReservation.length > 0) {
          const placeholders = buildPlaceholders(
            outputsToRemoveFromReservation.length
          );
          await conn.query(
            `DELETE FROM token_outputs WHERE id IN (${placeholders})`,
            outputsToRemoveFromReservation
          );

          const [emptyRows] = await conn.query(
            `SELECT r.id FROM token_reservations r
             LEFT JOIN token_outputs o ON o.reservation_id = r.id
             WHERE o.id IS NULL`
          );
          const emptyIds = emptyRows.map((r) => r.id);
          if (emptyIds.length > 0) {
            const emptyPlaceholders = buildPlaceholders(emptyIds.length);
            await conn.query(
              `DELETE FROM token_reservations WHERE id IN (${emptyPlaceholders})`,
              emptyIds
            );
          }
        }

        const [reservedOutputRows] = await conn.query(
          "SELECT id FROM token_outputs WHERE reservation_id IS NOT NULL"
        );
        const reservedOutputIds = new Set(reservedOutputRows.map((r) => r.id));

        await conn.query(
          `DELETE FROM token_metadata
           WHERE identifier NOT IN (
             SELECT DISTINCT token_identifier FROM token_outputs
           )`
        );

        for (const to of tokenOutputs) {
          await this._upsertMetadata(conn, to.metadata);

          for (const output of to.outputs) {
            if (
              reservedOutputIds.has(output.output.id) ||
              spentIds.has(output.output.id)
            ) {
              continue;
            }
            await this._insertSingleOutput(
              conn,
              to.metadata.identifier,
              output
            );
          }
        }
      });
    } catch (error) {
      if (error instanceof TokenStoreError) throw error;
      throw new TokenStoreError(
        `Failed to set token outputs: ${error.message}`,
        error
      );
    }
  }

  /**
   * Returns the spendable per-token balances aggregated server-side.
   * Each entry includes full token metadata + the available + swap-reserved sum.
   * Matches the in-memory default impl which returns all tokens that have
   * at least one output (including zero spendable balance).
   * @returns {Promise<Array<{metadata: object, balance: string}>>}
   */
  async getTokenBalances() {
    try {
      const [rows] = await this.pool.query(`
        SELECT m.identifier, m.issuer_public_key, m.name, m.ticker, m.decimals,
               m.max_supply, m.is_freezable, m.creation_entity_public_key,
               CAST(COALESCE(SUM(
                 CASE
                   WHEN o.reservation_id IS NULL THEN CAST(o.token_amount AS DECIMAL(65,0))
                   WHEN r.purpose = 'Swap' THEN CAST(o.token_amount AS DECIMAL(65,0))
                   ELSE 0
                 END
               ), 0) AS CHAR) AS balance
        FROM token_metadata m
        JOIN token_outputs o ON o.token_identifier = m.identifier
        LEFT JOIN token_reservations r ON o.reservation_id = r.id
        GROUP BY m.identifier, m.issuer_public_key, m.name, m.ticker,
                 m.decimals, m.max_supply, m.is_freezable, m.creation_entity_public_key
      `);
      return rows.map((row) => ({
        metadata: {
          identifier: row.identifier,
          issuerPublicKey: row.issuer_public_key,
          name: row.name,
          ticker: row.ticker,
          decimals: row.decimals,
          maxSupply: row.max_supply,
          isFreezable: toBool(row.is_freezable) ?? false,
          creationEntityPublicKey: row.creation_entity_public_key || null,
        },
        balance: row.balance,
      }));
    } catch (error) {
      throw new TokenStoreError(
        `Failed to get token balances: ${error.message}`,
        error
      );
    }
  }

  async listTokensOutputs() {
    try {
      const [rows] = await this.pool.query(
        `SELECT m.identifier, m.issuer_public_key, m.name, m.ticker, m.decimals,
                m.max_supply, m.is_freezable, m.creation_entity_public_key,
                o.id AS output_id, o.owner_public_key, o.revocation_commitment,
                o.withdraw_bond_sats, o.withdraw_relative_block_locktime,
                o.token_public_key, o.token_amount, o.token_identifier,
                o.prev_tx_hash, o.prev_tx_vout, o.reservation_id,
                r.purpose
         FROM token_metadata m
         LEFT JOIN token_outputs o ON o.token_identifier = m.identifier
         LEFT JOIN token_reservations r ON o.reservation_id = r.id
         ORDER BY m.identifier, CAST(o.token_amount AS DECIMAL(65,0)) ASC`
      );

      const map = new Map();

      for (const row of rows) {
        if (!map.has(row.identifier)) {
          map.set(row.identifier, {
            metadata: this._metadataFromRow(row),
            available: [],
            reservedForPayment: [],
            reservedForSwap: [],
          });
        }

        const entry = map.get(row.identifier);

        if (!row.output_id) {
          continue;
        }

        const output = this._outputFromRow(row);

        if (row.purpose === "Payment") {
          entry.reservedForPayment.push(output);
        } else if (row.purpose === "Swap") {
          entry.reservedForSwap.push(output);
        } else {
          entry.available.push(output);
        }
      }

      return Array.from(map.values());
    } catch (error) {
      if (error instanceof TokenStoreError) throw error;
      throw new TokenStoreError(
        `Failed to list token outputs: ${error.message}`,
        error
      );
    }
  }

  async getTokenOutputs(filter) {
    try {
      let whereClause;
      let param;

      if (filter.type === "identifier") {
        whereClause = "m.identifier = ?";
        param = filter.identifier;
      } else if (filter.type === "issuerPublicKey") {
        whereClause = "m.issuer_public_key = ?";
        param = filter.issuerPublicKey;
      } else {
        throw new TokenStoreError(`Unknown filter type: ${filter.type}`);
      }

      const [rows] = await this.pool.query(
        `SELECT m.identifier, m.issuer_public_key, m.name, m.ticker, m.decimals,
                m.max_supply, m.is_freezable, m.creation_entity_public_key,
                o.id AS output_id, o.owner_public_key, o.revocation_commitment,
                o.withdraw_bond_sats, o.withdraw_relative_block_locktime,
                o.token_public_key, o.token_amount, o.token_identifier,
                o.prev_tx_hash, o.prev_tx_vout, o.reservation_id,
                r.purpose
         FROM token_metadata m
         LEFT JOIN token_outputs o ON o.token_identifier = m.identifier
         LEFT JOIN token_reservations r ON o.reservation_id = r.id
         WHERE ${whereClause}
         ORDER BY CAST(o.token_amount AS DECIMAL(65,0)) ASC`,
        [param]
      );

      if (rows.length === 0) {
        throw new TokenStoreError("Token outputs not found");
      }

      const metadata = this._metadataFromRow(rows[0]);
      const entry = {
        metadata,
        available: [],
        reservedForPayment: [],
        reservedForSwap: [],
      };

      for (const row of rows) {
        if (!row.output_id) {
          continue;
        }

        const output = this._outputFromRow(row);

        if (row.purpose === "Payment") {
          entry.reservedForPayment.push(output);
        } else if (row.purpose === "Swap") {
          entry.reservedForSwap.push(output);
        } else {
          entry.available.push(output);
        }
      }

      return entry;
    } catch (error) {
      if (error instanceof TokenStoreError) throw error;
      throw new TokenStoreError(
        `Failed to get token outputs: ${error.message}`,
        error
      );
    }
  }

  async insertTokenOutputs(tokenOutputs) {
    try {
      const conn = await this.pool.getConnection();
      try {
        await conn.beginTransaction();

        await this._upsertMetadata(conn, tokenOutputs.metadata);

        const outputIds = tokenOutputs.outputs.map((o) => o.output.id);
        if (outputIds.length > 0) {
          const placeholders = buildPlaceholders(outputIds.length);
          await conn.query(
            `DELETE FROM token_spent_outputs WHERE output_id IN (${placeholders})`,
            outputIds
          );
        }

        for (const output of tokenOutputs.outputs) {
          await this._insertSingleOutput(
            conn,
            tokenOutputs.metadata.identifier,
            output
          );
        }

        await conn.commit();
      } catch (error) {
        await conn.rollback().catch(() => {});
        throw error;
      } finally {
        conn.release();
      }
    } catch (error) {
      if (error instanceof TokenStoreError) throw error;
      throw new TokenStoreError(
        `Failed to insert token outputs: ${error.message}`,
        error
      );
    }
  }

  async reserveTokenOutputs(
    tokenIdentifier,
    target,
    purpose,
    preferredOutputs,
    selectionStrategy
  ) {
    try {
      return await this._withWriteTransaction(async (conn) => {
        if (
          target.type === "minTotalValue" &&
          (!target.value || target.value === "0")
        ) {
          throw new TokenStoreError(
            "Amount to reserve must be greater than zero"
          );
        }
        if (
          target.type === "maxOutputCount" &&
          (!target.value || target.value === 0)
        ) {
          throw new TokenStoreError(
            "Count to reserve must be greater than zero"
          );
        }

        const [metadataRows] = await conn.query(
          "SELECT * FROM token_metadata WHERE identifier = ?",
          [tokenIdentifier]
        );

        if (metadataRows.length === 0) {
          throw new TokenStoreError(
            `Token outputs not found for identifier: ${tokenIdentifier}`
          );
        }

        const metadata = this._metadataFromRow(metadataRows[0]);

        const [outputRows] = await conn.query(
          `SELECT o.id AS output_id, o.owner_public_key, o.revocation_commitment,
                  o.withdraw_bond_sats, o.withdraw_relative_block_locktime,
                  o.token_public_key, o.token_amount, o.token_identifier,
                  o.prev_tx_hash, o.prev_tx_vout
           FROM token_outputs o
           WHERE o.token_identifier = ? AND o.reservation_id IS NULL`,
          [tokenIdentifier]
        );

        let outputs = outputRows.map((row) => this._outputFromRow(row));

        if (preferredOutputs && preferredOutputs.length > 0) {
          const preferredIds = new Set(
            preferredOutputs.map((p) => p.output.id)
          );
          outputs = outputs.filter((o) => preferredIds.has(o.output.id));
        }

        let selectedOutputs;

        if (target.type === "minTotalValue") {
          const amount = BigInt(target.value);

          const totalAvailable = outputs.reduce(
            (sum, o) => sum + BigInt(o.output.tokenAmount),
            0n
          );
          if (totalAvailable < amount) {
            throw new TokenStoreError("InsufficientFunds");
          }

          const exactMatch = outputs.find(
            (o) => BigInt(o.output.tokenAmount) === amount
          );
          if (exactMatch) {
            selectedOutputs = [exactMatch];
          } else {
            if (selectionStrategy === "LargestFirst") {
              outputs.sort(
                (a, b) =>
                  Number(
                    BigInt(b.output.tokenAmount) - BigInt(a.output.tokenAmount)
                  )
              );
            } else {
              outputs.sort(
                (a, b) =>
                  Number(
                    BigInt(a.output.tokenAmount) - BigInt(b.output.tokenAmount)
                  )
              );
            }

            selectedOutputs = [];
            let remaining = amount;
            for (const output of outputs) {
              if (remaining <= 0n) break;
              selectedOutputs.push(output);
              remaining -= BigInt(output.output.tokenAmount);
            }
            if (remaining > 0n) {
              throw new TokenStoreError("InsufficientFunds");
            }
          }
        } else if (target.type === "maxOutputCount") {
          const count = target.value;

          if (selectionStrategy === "LargestFirst") {
            outputs.sort(
              (a, b) =>
                Number(
                  BigInt(b.output.tokenAmount) - BigInt(a.output.tokenAmount)
                )
            );
          } else {
            outputs.sort(
              (a, b) =>
                Number(
                  BigInt(a.output.tokenAmount) - BigInt(b.output.tokenAmount)
                )
            );
          }

          selectedOutputs = outputs.slice(0, count);
        } else {
          throw new TokenStoreError(`Unknown target type: ${target.type}`);
        }

        const reservationId = this._generateId();

        await conn.query(
          "INSERT INTO token_reservations (id, purpose) VALUES (?, ?)",
          [reservationId, purpose]
        );

        const selectedIds = selectedOutputs.map((o) => o.output.id);
        if (selectedIds.length > 0) {
          const placeholders = buildPlaceholders(selectedIds.length);
          await conn.query(
            `UPDATE token_outputs SET reservation_id = ? WHERE id IN (${placeholders})`,
            [reservationId, ...selectedIds]
          );
        }

        return {
          id: reservationId,
          tokenOutputs: { metadata, outputs: selectedOutputs },
        };
      });
    } catch (error) {
      if (error instanceof TokenStoreError) throw error;
      throw new TokenStoreError(
        `Failed to reserve token outputs: ${error.message}`,
        error
      );
    }
  }

  async cancelReservation(id) {
    try {
      await this._withTransaction(async (conn) => {
        await conn.query(
          "UPDATE token_outputs SET reservation_id = NULL WHERE reservation_id = ?",
          [id]
        );
        await conn.query("DELETE FROM token_reservations WHERE id = ?", [id]);
      });
    } catch (error) {
      if (error instanceof TokenStoreError) throw error;
      throw new TokenStoreError(
        `Failed to cancel reservation '${id}': ${error.message}`,
        error
      );
    }
  }

  async finalizeReservation(id) {
    try {
      // _withWriteTransaction acquires the GET_LOCK so this serializes
      // against `setTokensOutputs`. Without it, a concurrent setTokensOutputs
      // could read token_spent_outputs before our marker commits and re-insert
      // the just-spent output as Available.
      await this._withWriteTransaction(async (conn) => {
        const [reservationRows] = await conn.query(
          "SELECT purpose FROM token_reservations WHERE id = ?",
          [id]
        );
        if (reservationRows.length === 0) {
          return;
        }
        const isSwap = reservationRows[0].purpose === "Swap";

        const [reservedRows] = await conn.query(
          "SELECT id FROM token_outputs WHERE reservation_id = ?",
          [id]
        );
        const reservedOutputIds = reservedRows.map((r) => r.id);

        if (reservedOutputIds.length > 0) {
          const valueClauses = new Array(reservedOutputIds.length)
            .fill("(?)")
            .join(", ");
          // Suppress duplicate-PK errors only.
          await conn.query(
            `INSERT INTO token_spent_outputs (output_id) VALUES ${valueClauses}
             ON DUPLICATE KEY UPDATE output_id = output_id`,
            reservedOutputIds
          );
        }

        await conn.query("DELETE FROM token_outputs WHERE reservation_id = ?", [
          id,
        ]);
        await conn.query("DELETE FROM token_reservations WHERE id = ?", [id]);

        if (isSwap) {
          await conn.query(
            "UPDATE token_swap_status SET last_completed_at = NOW(6) WHERE id = 1"
          );
        }

        await conn.query(
          `DELETE FROM token_metadata
           WHERE identifier NOT IN (
             SELECT DISTINCT token_identifier FROM token_outputs
           )`
        );
      });
    } catch (error) {
      if (error instanceof TokenStoreError) throw error;
      throw new TokenStoreError(
        `Failed to finalize reservation '${id}': ${error.message}`,
        error
      );
    }
  }

  async now() {
    try {
      const [rows] = await this.pool.query("SELECT NOW(6) AS now");
      const value = rows[0].now;
      if (value instanceof Date) return value.getTime();
      return new Date(value).getTime();
    } catch (error) {
      if (error instanceof TokenStoreError) throw error;
      throw new TokenStoreError(
        `Failed to get current time: ${error.message}`,
        error
      );
    }
  }

  // ===== Private Helpers =====

  _generateId() {
    if (typeof crypto !== "undefined" && crypto.randomUUID) {
      return crypto.randomUUID();
    }
    return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => {
      const r = (Math.random() * 16) | 0;
      const v = c === "x" ? r : (r & 0x3) | 0x8;
      return v.toString(16);
    });
  }

  async _cleanupStaleReservations(conn) {
    await conn.query(
      `DELETE FROM token_reservations
       WHERE created_at < DATE_SUB(NOW(6), INTERVAL ? SECOND)`,
      [RESERVATION_TIMEOUT_SECS]
    );
  }

  async _upsertMetadata(conn, metadata) {
    await conn.query(
      `INSERT INTO token_metadata
        (identifier, issuer_public_key, name, ticker, decimals, max_supply,
         is_freezable, creation_entity_public_key)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?)
       ON DUPLICATE KEY UPDATE
         issuer_public_key = VALUES(issuer_public_key),
         name = VALUES(name),
         ticker = VALUES(ticker),
         decimals = VALUES(decimals),
         max_supply = VALUES(max_supply),
         is_freezable = VALUES(is_freezable),
         creation_entity_public_key = VALUES(creation_entity_public_key)`,
      [
        metadata.identifier,
        metadata.issuerPublicKey,
        metadata.name,
        metadata.ticker,
        metadata.decimals,
        metadata.maxSupply,
        metadata.isFreezable ? 1 : 0,
        metadata.creationEntityPublicKey || null,
      ]
    );
  }

  async _insertSingleOutput(conn, tokenIdentifier, output) {
    // ON DUPLICATE KEY UPDATE id = id no-ops on the (id) primary key
    // conflict only — unlike INSERT IGNORE, FK / NOT NULL / type errors
    // still propagate.
    await conn.query(
      `INSERT INTO token_outputs
        (id, token_identifier, owner_public_key, revocation_commitment,
         withdraw_bond_sats, withdraw_relative_block_locktime,
         token_public_key, token_amount, prev_tx_hash, prev_tx_vout, added_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NOW(6))
       ON DUPLICATE KEY UPDATE id = id`,
      [
        output.output.id,
        tokenIdentifier,
        output.output.ownerPublicKey,
        output.output.revocationCommitment,
        output.output.withdrawBondSats,
        output.output.withdrawRelativeBlockLocktime,
        output.output.tokenPublicKey || null,
        output.output.tokenAmount,
        output.prevTxHash,
        output.prevTxVout,
      ]
    );
  }

  _metadataFromRow(row) {
    return {
      identifier: row.identifier,
      issuerPublicKey: row.issuer_public_key,
      name: row.name,
      ticker: row.ticker,
      decimals: row.decimals,
      maxSupply: row.max_supply,
      isFreezable: toBool(row.is_freezable) ?? false,
      creationEntityPublicKey: row.creation_entity_public_key || null,
    };
  }

  _outputFromRow(row) {
    return {
      output: {
        id: row.output_id,
        ownerPublicKey: row.owner_public_key,
        revocationCommitment: row.revocation_commitment,
        withdrawBondSats: Number(row.withdraw_bond_sats),
        withdrawRelativeBlockLocktime: Number(
          row.withdraw_relative_block_locktime
        ),
        tokenPublicKey: row.token_public_key || null,
        tokenIdentifier: row.token_identifier || row.identifier,
        tokenAmount: row.token_amount,
      },
      prevTxHash: row.prev_tx_hash,
      prevTxVout: row.prev_tx_vout,
    };
  }
}

function createMysqlPool(config) {
  return mysql.createPool({
    uri: config.connectionString,
    connectionLimit: config.maxPoolSize,
    connectTimeout: (config.createTimeoutSecs || 0) * 1000 || 10000,
    idleTimeout: (config.recycleTimeoutSecs || 0) * 1000 || 10000,
    waitForConnections: true,
  });
}

async function createMysqlTokenStore(config, logger = null) {
  const pool = createMysqlPool(config);
  return createMysqlTokenStoreWithPool(pool, logger);
}

async function createMysqlTokenStoreWithPool(pool, logger = null) {
  const store = new MysqlTokenStore(pool, logger);
  await store.initialize();
  return store;
}

module.exports = {
  MysqlTokenStore,
  createMysqlTokenStore,
  createMysqlTokenStoreWithPool,
  TokenStoreError,
};
