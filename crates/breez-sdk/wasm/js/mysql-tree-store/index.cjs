/**
 * CommonJS implementation for Node.js MySQL Tree Store.
 *
 * Mirrors `postgres-tree-store/index.cjs` for MySQL 8.0+. See
 * `mysql-storage/index.cjs` for SQL translation rules. Notable differences:
 * - `pg_advisory_xact_lock` is transaction-scoped; MySQL `GET_LOCK` is
 *   session-scoped, so we acquire it on the connection, run the transaction,
 *   release it explicitly afterwards.
 * - `UNNEST(arr)` batch inserts → manually built `VALUES (?,…), (?,…)`.
 * - `ANY(arr)` IN-array predicates → manually built `IN (?, ?, …)`.
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

const { TreeStoreError } = require("./errors.cjs");
const { MysqlTreeStoreMigrationManager } = require("./migrations.cjs");

/**
 * Named lock that serializes all tree store writes across processes.
 * Mirrors the Rust constant `TREE_STORE_WRITE_LOCK_NAME`.
 */
const TREE_STORE_WRITE_LOCK_NAME = "tree_store_write_lock";
/** Seconds to wait when acquiring the write lock. */
const WRITE_LOCK_TIMEOUT_SECS = 30;

const RESERVATION_TIMEOUT_SECS = 300;
const SPENT_MARKER_CLEANUP_THRESHOLD_MS = 5 * 60 * 1000;

/** mysql2 may return JSON columns as either parsed objects or raw strings. */
function parseJson(value) {
  if (value == null) return null;
  if (typeof value === "string") return JSON.parse(value);
  return value;
}

/** Normalize MySQL's TINYINT(1) to a JS boolean. */
function toBool(value) {
  if (value == null) return null;
  if (typeof value === "boolean") return value;
  return value === 1 || value === "1" || value === true;
}

function buildPlaceholders(n) {
  return new Array(n).fill("?").join(", ");
}

class MysqlTreeStore {
  constructor(pool, logger = null) {
    this.pool = pool;
    this.logger = logger;
  }

  async initialize() {
    try {
      const migrationManager = new MysqlTreeStoreMigrationManager(this.logger);
      await migrationManager.migrate(this.pool);
      return this;
    } catch (error) {
      throw new TreeStoreError(
        `Failed to initialize MySQL tree store: ${error.message}`,
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
   * Run a function inside a transaction, holding the named write lock for the
   * duration. Reserved for operations whose correctness depends on serializing
   * the available-leaf set (`tryReserveLeaves`, `setLeaves`).
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
        [TREE_STORE_WRITE_LOCK_NAME, WRITE_LOCK_TIMEOUT_SECS]
      );
      if (!lockRows || lockRows[0].acquired !== 1) {
        throw new TreeStoreError(
          `Failed to acquire tree store write lock within ${WRITE_LOCK_TIMEOUT_SECS}s`
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
          .query("SELECT RELEASE_LOCK(?)", [TREE_STORE_WRITE_LOCK_NAME])
          .catch(() => {});
      }
      conn.release();
    }
  }

  /**
   * Run a function inside a transaction without the advisory lock. Used by
   * operations scoped to a single reservation_id (`addLeaves`,
   * `cancelReservation`, `finalizeReservation`, `updateReservation`) where
   * row-level FK + InnoDB MVCC suffice and the global lock would only add
   * contention.
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

  // ===== TreeStore Methods =====

  async addLeaves(leaves) {
    try {
      if (!leaves || leaves.length === 0) {
        return;
      }

      await this._withTransaction(async (conn) => {
        const leafIds = leaves.map((l) => l.id);
        await this._batchRemoveSpentLeaves(conn, leafIds);
        await this._batchUpsertLeaves(conn, leaves, false, null);
      });
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to add leaves: ${error.message}`,
        error
      );
    }
  }

  /**
   * Returns the wallet's spendable balance (available + missing-from-operators
   * + swap-reserved). Aggregated server-side so we don't fetch every leaf.
   * @returns {Promise<bigint>}
   */
  async getAvailableBalance() {
    try {
      const [rows] = await this.pool.query(`
        SELECT COALESCE(SUM(l.value), 0) AS balance
        FROM tree_leaves l
        LEFT JOIN tree_reservations r ON l.reservation_id = r.id
        WHERE
          (l.reservation_id IS NULL AND l.status = 'Available')
          OR r.purpose = 'Swap'
      `);
      return BigInt(rows[0].balance);
    } catch (error) {
      throw new TreeStoreError(
        `Failed to get available balance: ${error.message}`,
        error
      );
    }
  }

  async getLeaves() {
    try {
      const [rows] = await this.pool.query(`
        SELECT l.id, l.status, l.is_missing_from_operators, l.data,
               l.reservation_id, r.purpose
        FROM tree_leaves l
        LEFT JOIN tree_reservations r ON l.reservation_id = r.id
      `);

      const available = [];
      const notAvailable = [];
      const availableMissingFromOperators = [];
      const reservedForPayment = [];
      const reservedForSwap = [];

      for (const row of rows) {
        const node = parseJson(row.data);

        if (row.purpose) {
          if (row.purpose === "Payment") {
            reservedForPayment.push(node);
          } else if (row.purpose === "Swap") {
            reservedForSwap.push(node);
          }
        } else if (toBool(row.is_missing_from_operators)) {
          if (node.status === "Available") {
            availableMissingFromOperators.push(node);
          }
        } else if (node.status === "Available") {
          available.push(node);
        } else {
          notAvailable.push(node);
        }
      }

      return {
        available,
        notAvailable,
        availableMissingFromOperators,
        reservedForPayment,
        reservedForSwap,
      };
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to get leaves: ${error.message}`,
        error
      );
    }
  }

  /**
   * Set leaves from a refresh operation.
   * @param {Array} leaves - Available leaves from operators
   * @param {Array} missingLeaves - Leaves missing from some operators
   * @param {number} refreshStartedAtMs - Epoch milliseconds when refresh started
   */
  async setLeaves(leaves, missingLeaves, refreshStartedAtMs) {
    try {
      await this._withWriteTransaction(async (conn) => {
        const refreshTimestamp = new Date(refreshStartedAtMs);

        // Drop expired reservations BEFORE evaluating has_active_swap.
        await this._cleanupStaleReservations(conn);

        const [swapRows] = await conn.query(
          `SELECT
            (SELECT EXISTS(SELECT 1 FROM tree_reservations WHERE purpose = 'Swap')) AS has_active_swap,
            COALESCE(
              (SELECT (last_completed_at >= ?) FROM tree_swap_status WHERE id = 1),
              0
            ) AS swap_completed_during_refresh`,
          [refreshTimestamp]
        );
        const hasActiveSwap = !!swapRows[0].has_active_swap;
        const swapCompletedDuringRefresh = !!swapRows[0].swap_completed_during_refresh;

        if (hasActiveSwap || swapCompletedDuringRefresh) {
          return;
        }

        await this._cleanupSpentMarkers(conn, refreshTimestamp);

        const [spentRows] = await conn.query(
          "SELECT leaf_id FROM tree_spent_leaves WHERE spent_at >= ?",
          [refreshTimestamp]
        );
        const spentIds = new Set(spentRows.map((r) => r.leaf_id));

        await conn.query(
          "DELETE FROM tree_leaves WHERE reservation_id IS NULL AND added_at < ?",
          [refreshTimestamp]
        );

        await this._batchUpsertLeaves(conn, leaves, false, spentIds);
        await this._batchUpsertLeaves(conn, missingLeaves, true, spentIds);
      });
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to set leaves: ${error.message}`,
        error
      );
    }
  }

  async cancelReservation(id, leavesToKeep) {
    try {
      await this._withTransaction(async (conn) => {
        const [existsRows] = await conn.query(
          "SELECT id FROM tree_reservations WHERE id = ?",
          [id]
        );

        if (existsRows.length === 0) {
          return;
        }

        await conn.query("DELETE FROM tree_leaves WHERE reservation_id = ?", [
          id,
        ]);
        await conn.query("DELETE FROM tree_reservations WHERE id = ?", [id]);

        if (leavesToKeep && leavesToKeep.length > 0) {
          await this._batchUpsertLeaves(conn, leavesToKeep, false, null);
        }
      });
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to cancel reservation '${id}': ${error.message}`,
        error
      );
    }
  }

  async finalizeReservation(id, newLeaves) {
    try {
      await this._withTransaction(async (conn) => {
        const [resRows] = await conn.query(
          "SELECT id, purpose FROM tree_reservations WHERE id = ?",
          [id]
        );

        let isSwap = false;
        if (resRows.length > 0) {
          isSwap = resRows[0].purpose === "Swap";
          const [leafRows] = await conn.query(
            "SELECT id FROM tree_leaves WHERE reservation_id = ?",
            [id]
          );
          const reservedLeafIds = leafRows.map((r) => r.id);
          await this._batchInsertSpentLeaves(conn, reservedLeafIds);
          await conn.query(
            "DELETE FROM tree_leaves WHERE reservation_id = ?",
            [id]
          );
          await conn.query("DELETE FROM tree_reservations WHERE id = ?", [id]);
        }

        if (newLeaves && newLeaves.length > 0) {
          await this._batchUpsertLeaves(conn, newLeaves, false, null);
        }

        if (isSwap && newLeaves && newLeaves.length > 0) {
          await conn.query(
            "UPDATE tree_swap_status SET last_completed_at = NOW(6) WHERE id = 1"
          );
        }
      });
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to finalize reservation '${id}': ${error.message}`,
        error
      );
    }
  }

  async tryReserveLeaves(targetAmounts, exactOnly, purpose) {
    try {
      return await this._withWriteTransaction(async (conn) => {
        const targetAmount = targetAmounts ? this._totalSats(targetAmounts) : 0;
        const maxTarget = this._maxTargetForPrefilter(targetAmounts);

        // True total available across ALL eligible leaves — required for the
        // WaitForPending decision. Must NOT be derived from the prefiltered
        // slim set since the prefilter excludes big leaves.
        const [totalRows] = await conn.query(
          `SELECT COALESCE(SUM(value), 0) AS total
           FROM tree_leaves
           WHERE status = 'Available'
             AND is_missing_from_operators = 0
             AND reservation_id IS NULL`
        );
        const available = Number(totalRows[0].total);

        // Slim projection: only (id, value) for leaves the selection might use.
        // Includes all leaves with value <= maxTarget plus the smallest leaf
        // with value > maxTarget (covers the minimum-amount fallback case
        // where one larger leaf is sufficient).
        const [slimRows] = await conn.query(
          `SELECT id, value
           FROM tree_leaves
           WHERE status = 'Available'
             AND is_missing_from_operators = 0
             AND reservation_id IS NULL
             AND (
               value <= ?
               OR id = (
                 SELECT id FROM (
                   SELECT id FROM tree_leaves
                   WHERE status = 'Available'
                     AND is_missing_from_operators = 0
                     AND reservation_id IS NULL
                     AND value > ?
                   ORDER BY value
                   LIMIT 1
                 ) AS smallest_over
               )
             )`,
          [maxTarget, maxTarget]
        );

        const slimLeaves = slimRows.map((r) => ({
          id: r.id,
          value: Number(r.value),
        }));

        const pending = await this._calculatePendingBalance(conn);

        // Try exact selection on slim leaves — selection only reads .id/.value
        const selected = this._selectLeavesByTargetAmounts(
          slimLeaves,
          targetAmounts
        );

        if (selected !== null) {
          if (selected.length === 0) {
            throw new TreeStoreError("NonReservableLeaves");
          }

          const fullLeaves = await this._fetchFullLeavesByIds(
            conn,
            selected.map((l) => l.id)
          );
          const reservationId = this._generateId();
          await this._createReservation(
            conn,
            reservationId,
            fullLeaves,
            purpose,
            0
          );

          return {
            type: "success",
            reservation: { id: reservationId, leaves: fullLeaves },
          };
        }

        if (!exactOnly) {
          const minSelected = this._selectLeavesByMinimumAmount(
            slimLeaves,
            targetAmount
          );
          if (minSelected !== null) {
            const fullLeaves = await this._fetchFullLeavesByIds(
              conn,
              minSelected.map((l) => l.id)
            );
            const reservedAmount = fullLeaves.reduce(
              (sum, l) => sum + l.value,
              0
            );
            const pendingChange =
              reservedAmount > targetAmount && targetAmount > 0
                ? reservedAmount - targetAmount
                : 0;

            const reservationId = this._generateId();
            await this._createReservation(
              conn,
              reservationId,
              fullLeaves,
              purpose,
              pendingChange
            );

            return {
              type: "success",
              reservation: { id: reservationId, leaves: fullLeaves },
            };
          }
        }

        if (available + pending >= targetAmount) {
          return {
            type: "waitForPending",
            needed: targetAmount,
            available,
            pending,
          };
        }

        return { type: "insufficientFunds" };
      });
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to try reserve leaves: ${error.message}`,
        error
      );
    }
  }

  async now() {
    try {
      const [rows] = await this.pool.query("SELECT NOW(6) AS now");
      const value = rows[0].now;
      // mysql2 typically returns DATETIME as a JS Date when dateStrings is false (default).
      if (value instanceof Date) return value.getTime();
      return new Date(value).getTime();
    } catch (error) {
      throw new TreeStoreError(
        `Failed to get current time: ${error.message}`,
        error
      );
    }
  }

  async updateReservation(reservationId, reservedLeaves, changeLeaves) {
    try {
      return await this._withTransaction(async (conn) => {
        const [existsRows] = await conn.query(
          "SELECT id FROM tree_reservations WHERE id = ?",
          [reservationId]
        );

        if (existsRows.length === 0) {
          throw new TreeStoreError(`Reservation ${reservationId} not found`);
        }

        const [oldLeafRows] = await conn.query(
          "SELECT id FROM tree_leaves WHERE reservation_id = ?",
          [reservationId]
        );
        const oldLeafIds = oldLeafRows.map((r) => r.id);

        await this._batchInsertSpentLeaves(conn, oldLeafIds);
        await conn.query(
          "DELETE FROM tree_leaves WHERE reservation_id = ?",
          [reservationId]
        );

        await this._batchUpsertLeaves(conn, changeLeaves, false, null);
        await this._batchUpsertLeaves(conn, reservedLeaves, false, null);

        const reservedLeafIds = reservedLeaves.map((l) => l.id);
        await this._batchSetReservationId(conn, reservationId, reservedLeafIds);

        await conn.query(
          "UPDATE tree_reservations SET pending_change_amount = 0 WHERE id = ?",
          [reservationId]
        );

        return { id: reservationId, leaves: reservedLeaves };
      });
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to update reservation '${reservationId}': ${error.message}`,
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

  _totalSats(targetAmounts) {
    if (targetAmounts.type === "amountAndFee") {
      return targetAmounts.amountSats + (targetAmounts.feeSats || 0);
    }
    if (targetAmounts.type === "exactDenominations") {
      return targetAmounts.denominations.reduce((sum, d) => sum + d, 0);
    }
    return 0;
  }

  /**
   * Largest single value the selection algorithm could possibly need.
   * For an unbounded target we have to return all leaves (no prefilter).
   */
  _maxTargetForPrefilter(targetAmounts) {
    if (!targetAmounts) return Number.MAX_SAFE_INTEGER;
    if (targetAmounts.type === "amountAndFee") {
      return Math.max(targetAmounts.amountSats, targetAmounts.feeSats || 0);
    }
    if (targetAmounts.type === "exactDenominations") {
      return targetAmounts.denominations.reduce((m, v) => Math.max(m, v), 0);
    }
    return Number.MAX_SAFE_INTEGER;
  }

  /**
   * Pull the full `data` JSON for the leaves the selection algorithm picked.
   * Typically this is 1-3 rows even when the prefiltered set was thousands.
   */
  async _fetchFullLeavesByIds(conn, ids) {
    if (!ids || ids.length === 0) return [];
    const placeholders = ids.map(() => "?").join(", ");
    const [rows] = await conn.query(
      `SELECT data FROM tree_leaves WHERE id IN (${placeholders})`,
      ids
    );
    return rows.map((r) => parseJson(r.data));
  }

  _selectLeavesByTargetAmounts(leaves, targetAmounts) {
    if (!targetAmounts) {
      return [...leaves];
    }

    if (targetAmounts.type === "amountAndFee") {
      const amountLeaves = this._selectLeavesByExactAmount(
        leaves,
        targetAmounts.amountSats
      );
      if (amountLeaves === null) return null;

      if (targetAmounts.feeSats != null && targetAmounts.feeSats > 0) {
        const amountIds = new Set(amountLeaves.map((l) => l.id));
        const remaining = leaves.filter((l) => !amountIds.has(l.id));
        const feeLeaves = this._selectLeavesByExactAmount(
          remaining,
          targetAmounts.feeSats
        );
        if (feeLeaves === null) return null;
        return [...amountLeaves, ...feeLeaves];
      }

      return amountLeaves;
    }

    if (targetAmounts.type === "exactDenominations") {
      return this._selectLeavesByExactDenominations(
        leaves,
        targetAmounts.denominations
      );
    }

    return null;
  }

  _selectLeavesByExactAmount(leaves, targetAmount) {
    if (targetAmount === 0) return null;

    const totalAvailable = leaves.reduce((sum, l) => sum + l.value, 0);
    if (totalAvailable < targetAmount) return null;

    const single = leaves.find((l) => l.value === targetAmount);
    if (single) return [single];

    return this._findExactMultipleMatch(leaves, targetAmount);
  }

  _selectLeavesByExactDenominations(leaves, denominations) {
    const remaining = [...leaves];
    const selected = [];

    for (const denomination of denominations) {
      const idx = remaining.findIndex((l) => l.value === denomination);
      if (idx === -1) return null;
      selected.push(remaining[idx]);
      remaining.splice(idx, 1);
    }

    return selected;
  }

  _selectLeavesByMinimumAmount(leaves, targetAmount) {
    if (targetAmount === 0) return null;

    const totalAvailable = leaves.reduce((sum, l) => sum + l.value, 0);
    if (totalAvailable < targetAmount) return null;

    const result = [];
    let sum = 0;
    for (const leaf of leaves) {
      sum += leaf.value;
      result.push(leaf);
      if (sum >= targetAmount) break;
    }

    return sum >= targetAmount ? result : null;
  }

  _findExactMultipleMatch(leaves, targetAmount) {
    if (targetAmount === 0) return [];
    if (leaves.length === 0) return null;

    const result = this._greedyExactMatch(leaves, targetAmount);
    if (result) return result;

    const powerOfTwoLeaves = leaves.filter((l) => this._isPowerOfTwo(l.value));
    if (powerOfTwoLeaves.length === leaves.length) return null;

    return this._greedyExactMatch(powerOfTwoLeaves, targetAmount);
  }

  _greedyExactMatch(leaves, targetAmount) {
    const sorted = [...leaves].sort((a, b) => b.value - a.value);
    const result = [];
    let remaining = targetAmount;

    for (const leaf of sorted) {
      if (leaf.value > remaining) continue;
      remaining -= leaf.value;
      result.push(leaf);
      if (remaining === 0) return result;
    }

    return null;
  }

  _isPowerOfTwo(value) {
    return value > 0 && (value & (value - 1)) === 0;
  }

  async _calculatePendingBalance(conn) {
    const [rows] = await conn.query(
      "SELECT COALESCE(SUM(pending_change_amount), 0) AS pending FROM tree_reservations"
    );
    return Number(rows[0].pending);
  }

  async _createReservation(conn, reservationId, leaves, purpose, pendingChange) {
    await conn.query(
      "INSERT INTO tree_reservations (id, purpose, pending_change_amount) VALUES (?, ?, ?)",
      [reservationId, purpose, pendingChange]
    );

    const leafIds = leaves.map((l) => l.id);
    await this._batchSetReservationId(conn, reservationId, leafIds);
  }

  async _batchUpsertLeaves(conn, leaves, isMissingFromOperators, skipIds) {
    if (!leaves || leaves.length === 0) return;

    const filtered = skipIds
      ? leaves.filter((l) => !skipIds.has(l.id))
      : leaves;

    if (filtered.length === 0) return;

    const valueClauses = new Array(filtered.length)
      .fill("(?, ?, ?, ?, ?, NOW(6))")
      .join(", ");
    const params = [];
    for (const leaf of filtered) {
      params.push(
        leaf.id,
        leaf.status,
        isMissingFromOperators ? 1 : 0,
        JSON.stringify(leaf),
        leaf.value
      );
    }

    await conn.query(
      `INSERT INTO tree_leaves (id, status, is_missing_from_operators, data, value, added_at)
       VALUES ${valueClauses}
       ON DUPLICATE KEY UPDATE
         status = VALUES(status),
         is_missing_from_operators = VALUES(is_missing_from_operators),
         data = VALUES(data),
         value = VALUES(value),
         added_at = NOW(6)`,
      params
    );
  }

  async _batchSetReservationId(conn, reservationId, leafIds) {
    if (leafIds.length === 0) return;

    const placeholders = buildPlaceholders(leafIds.length);
    await conn.query(
      `UPDATE tree_leaves SET reservation_id = ? WHERE id IN (${placeholders})`,
      [reservationId, ...leafIds]
    );
  }

  async _batchInsertSpentLeaves(conn, leafIds) {
    if (leafIds.length === 0) return;

    const valueClauses = new Array(leafIds.length).fill("(?)").join(", ");
    await conn.query(
      `INSERT IGNORE INTO tree_spent_leaves (leaf_id) VALUES ${valueClauses}`,
      leafIds
    );
  }

  async _batchRemoveSpentLeaves(conn, leafIds) {
    if (leafIds.length === 0) return;

    const placeholders = buildPlaceholders(leafIds.length);
    await conn.query(
      `DELETE FROM tree_spent_leaves WHERE leaf_id IN (${placeholders})`,
      leafIds
    );
  }

  async _cleanupStaleReservations(conn) {
    await conn.query(
      `DELETE FROM tree_reservations
       WHERE created_at < DATE_SUB(NOW(6), INTERVAL ? SECOND)`,
      [RESERVATION_TIMEOUT_SECS]
    );
  }

  async _cleanupSpentMarkers(conn, refreshTimestamp) {
    const cleanupCutoff = new Date(
      refreshTimestamp.getTime() - SPENT_MARKER_CLEANUP_THRESHOLD_MS
    );

    await conn.query("DELETE FROM tree_spent_leaves WHERE spent_at < ?", [
      cleanupCutoff,
    ]);
  }
}

/** Create a mysql2 pool from a config object. */
function createMysqlPool(config) {
  return mysql.createPool({
    uri: config.connectionString,
    connectionLimit: config.maxPoolSize,
    connectTimeout: (config.createTimeoutSecs || 0) * 1000 || 10000,
    idleTimeout: (config.recycleTimeoutSecs || 0) * 1000 || 10000,
    waitForConnections: true,
  });
}

async function createMysqlTreeStore(config, logger = null) {
  const pool = createMysqlPool(config);
  return createMysqlTreeStoreWithPool(pool, logger);
}

async function createMysqlTreeStoreWithPool(pool, logger = null) {
  const store = new MysqlTreeStore(pool, logger);
  await store.initialize();
  return store;
}

module.exports = {
  MysqlTreeStore,
  createMysqlTreeStore,
  createMysqlTreeStoreWithPool,
  TreeStoreError,
};
