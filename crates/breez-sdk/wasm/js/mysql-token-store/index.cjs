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

/**
 * Domain prefix mixed into the per-tenant `GET_LOCK` name. Distinct prefixes
 * guarantee that tree-store and token-store locks never collide.
 */
const TOKEN_STORE_LOCK_PREFIX = "breez-spark-sdk:token:";
/** Seconds to wait when acquiring the write lock. */
const WRITE_LOCK_TIMEOUT_SECS = 30;

const SPENT_MARKER_CLEANUP_THRESHOLD_MS = 5 * 60 * 1000;
const RESERVATION_TIMEOUT_SECS = 300;

/**
 * Derive a stable per-tenant lock name from a tenant identity pubkey. Hashes
 * a domain prefix together with the identity (SHA-256, first 8 bytes hex).
 */
function _identityLockName(prefix, identity) {
  const crypto = require("crypto");
  const hash = crypto.createHash("sha256");
  hash.update(prefix);
  hash.update(Buffer.from(identity));
  return prefix + hash.digest("hex").slice(0, 16);
}

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
  /**
   * @param {import('mysql2/promise').Pool} pool
   * @param {Buffer|Uint8Array} identity - 33-byte secp256k1 compressed pubkey
   *   identifying the tenant. All reads and writes are scoped by this.
   * @param {"Enforced"|"Disabled"} [foreignKeyMode="Enforced"] - whether
   *   migrations create database-enforced foreign keys.
   * @param {object} [logger]
   * @param {boolean} [runMigration=true] - whether to run schema migrations
   *   on initialize.
   */
  constructor(
    pool,
    identity,
    foreignKeyMode = "Enforced",
    logger = null,
    runMigration = true
  ) {
    if (!identity || identity.length !== 33) {
      throw new TokenStoreError(
        "tenant identity (33-byte secp256k1 pubkey) is required"
      );
    }
    this.pool = pool;
    this.identity = Buffer.from(identity);
    this.lockName = _identityLockName(TOKEN_STORE_LOCK_PREFIX, identity);
    this.foreignKeyMode = foreignKeyMode;
    this.logger = logger;
    this.runMigration = runMigration;
  }

  async initialize() {
    try {
      if (this.runMigration) {
        const migrationManager = new MysqlTokenStoreMigrationManager(
          this.logger,
          this.foreignKeyMode
        );
        await migrationManager.migrate(this.pool, this.identity);
      }
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
        [this.lockName, WRITE_LOCK_TIMEOUT_SECS]
      );
      if (!lockRows || lockRows[0].acquired !== 1) {
        throw new TokenStoreError(
          `Failed to acquire token store write lock within ${WRITE_LOCK_TIMEOUT_SECS}s`
        );
      }
      lockAcquired = true;

      await conn.query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED");
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
          .query("SELECT RELEASE_LOCK(?)", [this.lockName])
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
      await conn.query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED");
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
        // Drop expired reservations BEFORE evaluating has_active_swap, otherwise a stale
        // Swap reservation (from a crashed client or a swap whose finalize/cancel never
        // ran) keeps has_active_swap true forever, which makes setTokensOutputs
        // early-return and never reach any subsequent reconciliation. The reservation
        // pins itself in place and the local token-output set freezes.
        await this._cleanupStaleReservations(conn);

        const [swapRows] = await conn.query(
          `SELECT
            (SELECT EXISTS(SELECT 1 FROM brz_token_reservations WHERE user_id = ? AND purpose = 'Swap')) AS has_active_swap,
            COALESCE(
              (SELECT (last_completed_at >= ?) FROM brz_token_swap_status WHERE user_id = ?),
              0
            ) AS swap_completed`,
          [this.identity, refreshTimestamp, this.identity]
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
          "DELETE FROM brz_token_spent_outputs WHERE user_id = ? AND spent_at < ?",
          [this.identity, cleanupCutoff]
        );

        const [spentRows] = await conn.query(
          "SELECT prev_tx_hash, prev_tx_vout FROM brz_token_spent_outputs WHERE user_id = ? AND spent_at >= ?",
          [this.identity, refreshTimestamp]
        );
        const spentOutpoints = new Set(
          spentRows.map((r) => `${r.prev_tx_hash}:${r.prev_tx_vout}`)
        );

        await conn.query(
          "DELETE FROM brz_token_outputs WHERE user_id = ? AND reservation_id IS NULL AND added_at < ?",
          [this.identity, refreshTimestamp]
        );

        const incomingOutpoints = new Set();
        for (const to of tokenOutputs) {
          for (const o of to.outputs) {
            incomingOutpoints.add(`${o.prevTxHash}:${o.prevTxVout}`);
          }
        }

        const [reservedRows] = await conn.query(
          `SELECT r.id, o.prev_tx_hash, o.prev_tx_vout
           FROM brz_token_reservations r
           JOIN brz_token_outputs o
             ON o.reservation_id = r.id AND o.user_id = r.user_id
           WHERE r.user_id = ?`,
          [this.identity]
        );

        const reservationOutputs = new Map();
        for (const row of reservedRows) {
          if (!reservationOutputs.has(row.id)) {
            reservationOutputs.set(row.id, []);
          }
          reservationOutputs.get(row.id).push([row.prev_tx_hash, row.prev_tx_vout]);
        }

        const reservationsToDelete = [];
        const outpointsToRemoveFromReservation = [];
        for (const [reservationId, outpoints] of reservationOutputs) {
          const hasValid = outpoints.some(([h, v]) =>
            incomingOutpoints.has(`${h}:${v}`)
          );
          if (hasValid) {
            for (const [h, v] of outpoints) {
              if (!incomingOutpoints.has(`${h}:${v}`)) {
                outpointsToRemoveFromReservation.push([h, v]);
              }
            }
          } else {
            reservationsToDelete.push(reservationId);
          }
        }

        if (reservationsToDelete.length > 0) {
          const placeholders = buildPlaceholders(reservationsToDelete.length);
          await conn.query(
            `DELETE FROM brz_token_outputs WHERE user_id = ? AND reservation_id IN (${placeholders})`,
            [this.identity, ...reservationsToDelete]
          );
          await conn.query(
            `DELETE FROM brz_token_reservations WHERE user_id = ? AND id IN (${placeholders})`,
            [this.identity, ...reservationsToDelete]
          );
        }

        if (outpointsToRemoveFromReservation.length > 0) {
          const pairPlaceholders = outpointsToRemoveFromReservation
            .map(() => "(?, ?)")
            .join(", ");
          const params = [this.identity];
          for (const [h, v] of outpointsToRemoveFromReservation) {
            params.push(h, v);
          }
          await conn.query(
            `DELETE FROM brz_token_outputs WHERE user_id = ?
               AND (prev_tx_hash, prev_tx_vout) IN (${pairPlaceholders})`,
            params
          );

          const [emptyRows] = await conn.query(
            `SELECT r.id FROM brz_token_reservations r
             LEFT JOIN brz_token_outputs o
               ON o.reservation_id = r.id AND o.user_id = r.user_id
             WHERE r.user_id = ? AND o.prev_tx_hash IS NULL`,
            [this.identity]
          );
          const emptyIds = emptyRows.map((r) => r.id);
          if (emptyIds.length > 0) {
            const emptyPlaceholders = buildPlaceholders(emptyIds.length);
            await conn.query(
              `DELETE FROM brz_token_reservations WHERE user_id = ? AND id IN (${emptyPlaceholders})`,
              [this.identity, ...emptyIds]
            );
          }
        }

        const [reservedOutputRows] = await conn.query(
          "SELECT prev_tx_hash, prev_tx_vout FROM brz_token_outputs WHERE user_id = ? AND reservation_id IS NOT NULL",
          [this.identity]
        );
        const reservedOutpoints = new Set(
          reservedOutputRows.map((r) => `${r.prev_tx_hash}:${r.prev_tx_vout}`)
        );

        await conn.query(
          `DELETE FROM brz_token_metadata
           WHERE user_id = ?
             AND identifier NOT IN (
               SELECT DISTINCT token_identifier FROM brz_token_outputs WHERE user_id = ?
             )`,
          [this.identity, this.identity]
        );

        for (const to of tokenOutputs) {
          await this._upsertMetadata(conn, to.metadata);

          for (const output of to.outputs) {
            const outpoint = `${output.prevTxHash}:${output.prevTxVout}`;
            if (reservedOutpoints.has(outpoint) || spentOutpoints.has(outpoint)) {
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
      const [rows] = await this.pool.query(
        `SELECT m.identifier, m.issuer_public_key, m.name, m.ticker, m.decimals,
                m.max_supply, m.is_freezable, m.creation_entity_public_key,
                CAST(COALESCE(SUM(
                  CASE
                    WHEN o.reservation_id IS NULL THEN CAST(o.token_amount AS DECIMAL(65,0))
                    WHEN r.purpose = 'Swap' THEN CAST(o.token_amount AS DECIMAL(65,0))
                    ELSE 0
                  END
                ), 0) AS CHAR) AS balance
         FROM brz_token_metadata m
         JOIN brz_token_outputs o
           ON o.token_identifier = m.identifier AND o.user_id = m.user_id
         LEFT JOIN brz_token_reservations r
           ON o.reservation_id = r.id AND o.user_id = r.user_id
         WHERE m.user_id = ?
         GROUP BY m.identifier, m.issuer_public_key, m.name, m.ticker,
                  m.decimals, m.max_supply, m.is_freezable, m.creation_entity_public_key`,
        [this.identity]
      );
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
                o.owner_public_key, o.revocation_commitment,
                o.withdraw_bond_sats, o.withdraw_relative_block_locktime,
                o.token_public_key, o.token_amount, o.token_identifier,
                o.prev_tx_hash, o.prev_tx_vout, o.reservation_id,
                r.purpose
         FROM brz_token_metadata m
         LEFT JOIN brz_token_outputs o
           ON o.token_identifier = m.identifier AND o.user_id = m.user_id
         LEFT JOIN brz_token_reservations r
           ON o.reservation_id = r.id AND o.user_id = r.user_id
         WHERE m.user_id = ?
         ORDER BY m.identifier, CAST(o.token_amount AS DECIMAL(65,0)) ASC`,
        [this.identity]
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

        if (!row.prev_tx_hash) {
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
                o.owner_public_key, o.revocation_commitment,
                o.withdraw_bond_sats, o.withdraw_relative_block_locktime,
                o.token_public_key, o.token_amount, o.token_identifier,
                o.prev_tx_hash, o.prev_tx_vout, o.reservation_id,
                r.purpose
         FROM brz_token_metadata m
         LEFT JOIN brz_token_outputs o
           ON o.token_identifier = m.identifier AND o.user_id = m.user_id
         LEFT JOIN brz_token_reservations r
           ON o.reservation_id = r.id AND o.user_id = r.user_id
         WHERE m.user_id = ? AND ${whereClause}
         ORDER BY CAST(o.token_amount AS DECIMAL(65,0)) ASC`,
        [this.identity, param]
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
        if (!row.prev_tx_hash) {
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

  /**
   * Atomically remove spent outputs and insert new outputs.
   * @param {Array<[string, number]>} outputsToRemove - Array of [prevTxHash, prevTxVout] tuples
   * @param {Object|null} outputsToAdd - Token outputs to insert (with metadata)
   * @returns {Promise<void>}
   */
  async updateTokenOutputs(outputsToRemove, outputsToAdd) {
    try {
      // Serialize against the other token-store mutators (refresh, reservation,
      // finalization), which take the same per-user advisory lock.
      await this._withWriteTransaction(async (conn) => {
        // 1. Remove spent outputs and mark as spent.
        if (outputsToRemove && outputsToRemove.length > 0) {
          for (const [txHash, vout] of outputsToRemove) {
            const [result] = await conn.query(
              "DELETE FROM brz_token_outputs WHERE user_id = ? AND prev_tx_hash = ? AND prev_tx_vout = ?",
              [this.identity, txHash, vout]
            );
            if (result.affectedRows > 0) {
              await conn.query(
                "INSERT IGNORE INTO brz_token_spent_outputs (user_id, prev_tx_hash, prev_tx_vout, spent_at) VALUES (?, ?, ?, UTC_TIMESTAMP(6))",
                [this.identity, txHash, vout]
              );
            }
          }
        }

        // 2. Insert new outputs.
        if (outputsToAdd) {
          await this._upsertMetadata(conn, outputsToAdd.metadata);

          if (outputsToAdd.outputs.length > 0) {
            const pairPlaceholders = outputsToAdd.outputs
              .map(() => "(?, ?)")
              .join(", ");
            const params = [this.identity];
            for (const o of outputsToAdd.outputs) {
              params.push(o.prevTxHash, o.prevTxVout);
            }
            await conn.query(
              `DELETE FROM brz_token_spent_outputs WHERE user_id = ? AND (prev_tx_hash, prev_tx_vout) IN (${pairPlaceholders})`,
              params
            );
          }

          for (const output of outputsToAdd.outputs) {
            await this._insertSingleOutput(
              conn,
              outputsToAdd.metadata.identifier,
              output
            );
          }
        }
      });
    } catch (error) {
      if (error instanceof TokenStoreError) throw error;
      throw new TokenStoreError(
        `Failed to update token outputs: ${error.message}`,
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
          "SELECT * FROM brz_token_metadata WHERE user_id = ? AND identifier = ?",
          [this.identity, tokenIdentifier]
        );

        if (metadataRows.length === 0) {
          throw new TokenStoreError(
            `Token outputs not found for identifier: ${tokenIdentifier}`
          );
        }

        const metadata = this._metadataFromRow(metadataRows[0]);

        const [outputRows] = await conn.query(
          `SELECT o.owner_public_key, o.revocation_commitment,
                  o.withdraw_bond_sats, o.withdraw_relative_block_locktime,
                  o.token_public_key, o.token_amount, o.token_identifier,
                  o.prev_tx_hash, o.prev_tx_vout
           FROM brz_token_outputs o
           WHERE o.user_id = ? AND o.token_identifier = ? AND o.reservation_id IS NULL`,
          [this.identity, tokenIdentifier]
        );

        let outputs = outputRows.map((row) => this._outputFromRow(row));

        if (preferredOutputs && preferredOutputs.length > 0) {
          const preferredOutpoints = new Set(
            preferredOutputs.map((p) => `${p.prevTxHash}:${p.prevTxVout}`)
          );
          outputs = outputs.filter((o) =>
            preferredOutpoints.has(`${o.prevTxHash}:${o.prevTxVout}`)
          );
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
          "INSERT INTO brz_token_reservations (user_id, id, purpose, created_at) VALUES (?, ?, ?, UTC_TIMESTAMP(6))",
          [this.identity, reservationId, purpose]
        );

        if (selectedOutputs.length > 0) {
          const pairPlaceholders = selectedOutputs
            .map(() => "(?, ?)")
            .join(", ");
          const params = [reservationId, this.identity];
          for (const o of selectedOutputs) {
            params.push(o.prevTxHash, o.prevTxVout);
          }
          await conn.query(
            `UPDATE brz_token_outputs SET reservation_id = ? WHERE user_id = ?
               AND (prev_tx_hash, prev_tx_vout) IN (${pairPlaceholders})`,
            params
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
        // Clear reservation_id from outputs first — the composite FK uses NO
        // ACTION (a whole-row SET NULL would null user_id, which is NOT NULL).
        await conn.query(
          "UPDATE brz_token_outputs SET reservation_id = NULL WHERE user_id = ? AND reservation_id = ?",
          [this.identity, id]
        );
        await conn.query(
          "DELETE FROM brz_token_reservations WHERE user_id = ? AND id = ?",
          [this.identity, id]
        );
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
      // could read brz_token_spent_outputs before our marker commits and re-insert
      // the just-spent output as Available.
      await this._withWriteTransaction(async (conn) => {
        const [reservationRows] = await conn.query(
          "SELECT purpose FROM brz_token_reservations WHERE user_id = ? AND id = ?",
          [this.identity, id]
        );
        if (reservationRows.length === 0) {
          return;
        }
        const isSwap = reservationRows[0].purpose === "Swap";

        const [reservedRows] = await conn.query(
          "SELECT prev_tx_hash, prev_tx_vout FROM brz_token_outputs WHERE user_id = ? AND reservation_id = ?",
          [this.identity, id]
        );

        if (reservedRows.length > 0) {
          const valueClauses = new Array(reservedRows.length)
            .fill("(?, ?, ?, UTC_TIMESTAMP(6))")
            .join(", ");
          const params = [];
          for (const row of reservedRows) {
            params.push(this.identity, row.prev_tx_hash, row.prev_tx_vout);
          }
          // Suppress duplicate-PK errors only.
          await conn.query(
            `INSERT INTO brz_token_spent_outputs (user_id, prev_tx_hash, prev_tx_vout, spent_at) VALUES ${valueClauses}
             ON DUPLICATE KEY UPDATE prev_tx_hash = prev_tx_hash`,
            params
          );
        }

        await conn.query(
          "DELETE FROM brz_token_outputs WHERE user_id = ? AND reservation_id = ?",
          [this.identity, id]
        );
        await conn.query(
          "DELETE FROM brz_token_reservations WHERE user_id = ? AND id = ?",
          [this.identity, id]
        );

        // UPSERT so a tenant that joined after the multi-tenant migration
        // (and thus has no row) gets one created lazily.
        if (isSwap) {
          await conn.query(
            `INSERT INTO brz_token_swap_status (user_id, last_completed_at) VALUES (?, UTC_TIMESTAMP(6))
             ON DUPLICATE KEY UPDATE last_completed_at = VALUES(last_completed_at)`,
            [this.identity]
          );
        }

        await conn.query(
          `DELETE FROM brz_token_metadata
           WHERE user_id = ?
             AND identifier NOT IN (
               SELECT DISTINCT token_identifier FROM brz_token_outputs WHERE user_id = ?
             )`,
          [this.identity, this.identity]
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
      const [rows] = await this.pool.query("SELECT UTC_TIMESTAMP(6) AS now");
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

  /// Cleans up stale reservations for THIS tenant. Releases dependent outputs
  /// by clearing reservation_id first, then deletes the parent rows — the
  /// composite FK uses NO ACTION because column-list SET NULL would null
  /// user_id (NOT NULL).
  async _cleanupStaleReservations(conn) {
    await conn.query(
      `UPDATE brz_token_outputs SET reservation_id = NULL
       WHERE user_id = ?
         AND reservation_id IN (
           SELECT id FROM (
             SELECT id FROM brz_token_reservations
             WHERE user_id = ?
               AND created_at < DATE_SUB(UTC_TIMESTAMP(6), INTERVAL ? SECOND)
           ) AS stale
         )`,
      [this.identity, this.identity, RESERVATION_TIMEOUT_SECS]
    );
    await conn.query(
      `DELETE FROM brz_token_reservations
       WHERE user_id = ? AND created_at < DATE_SUB(UTC_TIMESTAMP(6), INTERVAL ? SECOND)`,
      [this.identity, RESERVATION_TIMEOUT_SECS]
    );
  }

  async _upsertMetadata(conn, metadata) {
    await conn.query(
      `INSERT INTO brz_token_metadata
        (user_id, identifier, issuer_public_key, name, ticker, decimals, max_supply,
         is_freezable, creation_entity_public_key)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
       ON DUPLICATE KEY UPDATE
         issuer_public_key = VALUES(issuer_public_key),
         name = VALUES(name),
         ticker = VALUES(ticker),
         decimals = VALUES(decimals),
         max_supply = VALUES(max_supply),
         is_freezable = VALUES(is_freezable),
         creation_entity_public_key = VALUES(creation_entity_public_key)`,
      [
        this.identity,
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
    // ON DUPLICATE KEY UPDATE prev_tx_hash = prev_tx_hash no-ops on the
    // (user_id, prev_tx_hash, prev_tx_vout) primary key conflict only — unlike
    // INSERT IGNORE, FK / NOT NULL / type errors still propagate.
    await conn.query(
      `INSERT INTO brz_token_outputs
        (user_id, token_identifier, owner_public_key, revocation_commitment,
         withdraw_bond_sats, withdraw_relative_block_locktime,
         token_public_key, token_amount, prev_tx_hash, prev_tx_vout, added_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, UTC_TIMESTAMP(6))
       ON DUPLICATE KEY UPDATE prev_tx_hash = prev_tx_hash`,
      [
        this.identity,
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
    // Serialize JS `Date` parameters as UTC strings rather than host-local
    // time. Paired with explicit `UTC_TIMESTAMP(6)` on the server side, this
    // keeps timestamp comparisons consistent regardless of the host TZ.
    timezone: "Z",
  });
}

/**
 * @param {object} config - MySQL configuration
 * @param {Buffer|Uint8Array} identity - 33-byte secp256k1 compressed pubkey
 *   identifying the tenant. All reads and writes are scoped by this.
 * @param {object} [logger]
 */
async function createMysqlTokenStore(config, identity, logger = null) {
  const pool = createMysqlPool(config);
  return createMysqlTokenStoreWithPool(
    pool,
    identity,
    config.foreignKeyMode || "Enforced",
    logger,
    config.runMigration !== false
  );
}

async function createMysqlTokenStoreWithPool(
  pool,
  identity,
  foreignKeyMode = "Enforced",
  logger = null,
  runMigration = true
) {
  const store = new MysqlTokenStore(
    pool,
    identity,
    foreignKeyMode,
    logger,
    runMigration
  );
  await store.initialize();
  return store;
}

module.exports = {
  MysqlTokenStore,
  createMysqlTokenStore,
  createMysqlTokenStoreWithPool,
  TokenStoreError,
};
