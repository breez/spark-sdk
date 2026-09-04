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
 * Domain prefix mixed into the per-tenant `GET_LOCK` name. Distinct prefixes
 * guarantee that tree-store and token-store locks never collide.
 */
const TREE_STORE_LOCK_PREFIX = "breez-spark-sdk:tree:";
/** Seconds to wait when acquiring the write lock. */
const WRITE_LOCK_TIMEOUT_SECS = 30;

const RESERVATION_TIMEOUT_SECS = 300;
const SPENT_MARKER_CLEANUP_THRESHOLD_MS = 5 * 60 * 1000;

/**
 * Leaves per INSERT when upserting a refreshed leaf set.
 *
 * A wallet can hold six figures of leaves, each serializing to a JSON blob
 * carrying up to five transactions. The upsert goes through conn.query(),
 * which interpolates its placeholders client-side, so the whole set would
 * otherwise be built in memory as one statement and sent as one packet,
 * against max_allowed_packet.
 */
const LEAF_UPSERT_CHUNK_SIZE = 1000;

/**
 * Ancestor rows per INSERT when storing exit chains.
 *
 * A wallet-wide chain backfill carries a row per leaf per ancestor, each a JSON
 * blob of up to five transactions. The insert goes through conn.query(), which
 * interpolates its placeholders client-side, so the whole set would otherwise be
 * built in memory as one statement and sent as one packet, against
 * max_allowed_packet.
 */
const ANCESTOR_INSERT_CHUNK_SIZE = 4096;

/**
 * Slim projection: only (id, value) for leaves the selection might use.
 * Includes all leaves with value <= the max target (covers exact-match + the
 * small-leaf accumulators for the minimum-amount path) plus the single
 * smallest leaf with a larger value (covers the minimum-amount fallback case
 * where one larger leaf is sufficient).
 * Params: user id, max target, user id, max target.
 */
const SLIM_LEAF_CANDIDATES_SQL = `SELECT id, value
  FROM brz_tree_leaves
  WHERE user_id = ?
    AND status = 'Available'
    AND is_missing_from_operators = 0
    AND is_deleted = 0
    AND reservation_id IS NULL
    AND (
      value <= ?
      OR id = (
        SELECT id FROM (
          SELECT id FROM brz_tree_leaves
          WHERE user_id = ?
            AND status = 'Available'
            AND is_missing_from_operators = 0
            AND is_deleted = 0
            AND reservation_id IS NULL
            AND value > ?
          ORDER BY value
          LIMIT 1
        ) AS smallest_over
      )
    )`;

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

/**
 * Pair a leaf with its ancestors (nearest first) by walking `parent_node_id`
 * through `nodes`. Returns null if the leaf itself is absent; stops at a gap or
 * cycle, returning a partial chain.
 * @param {Map<string, object>} nodes
 * @param {string} leafId
 * @returns {{leaf: object, ancestors: Array<object>}|null}
 */
function assembleExitChain(nodes, leafId) {
  const leaf = nodes.get(leafId);
  if (!leaf) return null;
  const ancestors = [];
  const visited = new Set([leafId]);
  let current = leaf.parent_node_id;
  while (current != null && !visited.has(current)) {
    visited.add(current);
    const node = nodes.get(current);
    if (!node) break;
    ancestors.push(node);
    current = node.parent_node_id;
  }
  return { leaf, ancestors };
}

class MysqlTreeStore {
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
      throw new TreeStoreError(
        "tenant identity (33-byte secp256k1 pubkey) is required"
      );
    }
    this.pool = pool;
    this.identity = Buffer.from(identity);
    this.lockName = _identityLockName(TREE_STORE_LOCK_PREFIX, identity);
    this.foreignKeyMode = foreignKeyMode;
    this.logger = logger;
    this.runMigration = runMigration;
  }

  async initialize() {
    try {
      if (this.runMigration) {
        const migrationManager = new MysqlTreeStoreMigrationManager(
          this.logger,
          this.foreignKeyMode
        );
        await migrationManager.migrate(this.pool, this.identity);
      }
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
   * duration. Used by every operation that mutates the leaf set or its
   * reservations.
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
        throw new TreeStoreError(
          `Failed to acquire tree store write lock within ${WRITE_LOCK_TIMEOUT_SECS}s`
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
   * `addLeaves` and by read-only queries (`trySelectLeaves`), where row-level
   * FK + InnoDB MVCC suffice and the global lock would only add contention.
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

  // ===== TreeStore Methods =====

  async addLeaves(leaves) {
    try {
      if (!leaves || leaves.length === 0) {
        return;
      }

      const leafNodes = leaves;
      await this._withTransaction(async (conn) => {
        const leafIds = leafNodes.map((l) => l.id);
        await this._batchRemoveSpentLeaves(conn, leafIds);
        await this._batchUpsertLeaves(conn, leafNodes, false, null);
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
   * Store the ancestor chain of each pedigree, leaving the leaf pool and any
   * spent marker untouched.
   * @param {Array} pedigrees - Array of LeafPedigree { leaf, ancestors }
   */
  async storeAncestors(pedigrees) {
    try {
      if (!pedigrees || pedigrees.length === 0) {
        return;
      }

      await this._withWriteTransaction(async (conn) => {
        // A leaf can be spent between its chain being resolved and this write, and
        // a chain is only ever removed with its leaf. Writing one for a leaf that is
        // already gone would leave it behind for good.
        const placeholders = buildPlaceholders(pedigrees.length);
        const [storedRows] = await conn.query(
          `SELECT id FROM brz_tree_leaves WHERE user_id = ? AND id IN (${placeholders})`,
          [this.identity, ...pedigrees.map((p) => p.leaf.id)]
        );
        const storedLeafIds = new Set(storedRows.map((row) => row.id));

        await this._batchUpsertAncestors(
          conn,
          pedigrees.filter((p) => storedLeafIds.has(p.leaf.id))
        );
      });
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to store ancestors: ${error.message}`,
        error
      );
    }
  }

  /**
   * Ids of the stored leaves whose chain cannot back an exit: the leaf has a
   * parent, and no ancestor row of its own holds that parent.
   * @returns {Promise<Array<string>>}
   */
  async leavesMissingExitChains() {
    try {
      // A stored chain runs from its leaf's parent to a root, so a leaf whose
      // chain holds the parent it has now is exitable. The join binds all three
      // primary key columns, making it one index probe per leaf. A leaf that is
      // itself a root needs no chain.
      const [rows] = await this.pool.query(
        `SELECT l.id
         FROM brz_tree_leaves l
         LEFT JOIN brz_tree_ancestors link
           ON link.user_id = l.user_id AND link.leaf_id = l.id
              AND link.id = l.parent_node_id
         WHERE l.user_id = ?
           AND l.parent_node_id IS NOT NULL
           AND link.leaf_id IS NULL`,
        [this.identity]
      );
      return rows.map((r) => r.id);
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to get leaves missing exit chains: ${error.message}`,
        error
      );
    }
  }

  /**
   * Reconstruct the exit chains for many leaves in one query, each as
   * { leaf, ancestors } with ancestors nearest first. A leaf absent from the store
   * is skipped; a chain that hits a gap comes back partial.
   * @param {Array<string>} leafIds
   * @returns {Promise<Array<{leaf: object, ancestors: Array<object>}>>}
   */
  async getExitChains(leafIds) {
    try {
      if (!leafIds || leafIds.length === 0) return [];
      // One query loads each requested leaf's own row plus its ancestor rows,
      // both tagged by the owning leaf id (a leaf's own row is tagged with its
      // own id). Reading them in two queries could pair a leaf with ancestors
      // from a different snapshot. Grouping by that tag keeps each leaf's node
      // set separate, so a node id stored under another leaf can never
      // cross-contaminate this one.
      const placeholders = buildPlaceholders(leafIds.length);
      const [rows] = await this.pool.query(
        `SELECT leaf_id, data FROM brz_tree_ancestors WHERE user_id = ? AND leaf_id IN (${placeholders})
         UNION ALL
         SELECT id AS leaf_id, data FROM brz_tree_leaves WHERE user_id = ? AND id IN (${placeholders})`,
        [this.identity, ...leafIds, this.identity, ...leafIds]
      );

      const nodesByLeaf = new Map();
      for (const r of rows) {
        let nodes = nodesByLeaf.get(r.leaf_id);
        if (!nodes) {
          nodes = new Map();
          nodesByLeaf.set(r.leaf_id, nodes);
        }
        const node = parseJson(r.data);
        nodes.set(node.id, node);
      }

      const result = [];
      for (const id of leafIds) {
        const nodes = nodesByLeaf.get(id);
        if (!nodes) continue;
        const pedigree = assembleExitChain(nodes, id);
        if (pedigree) result.push(pedigree);
      }
      return result;
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(`Failed to get exit chains: ${error.message}`, error);
    }
  }

  /**
   * Returns the wallet's spendable balance (available + missing-from-operators
   * + swap-reserved). Aggregated server-side so we don't fetch every leaf.
   * @returns {Promise<bigint>}
   */
  async getAvailableBalance() {
    try {
      const [rows] = await this.pool.query(
        `SELECT COALESCE(SUM(l.value), 0) AS balance
         FROM brz_tree_leaves l
         LEFT JOIN brz_tree_reservations r
           ON l.reservation_id = r.id AND l.user_id = r.user_id
         WHERE l.user_id = ?
           AND l.is_deleted = 0
           AND (
             (l.reservation_id IS NULL AND l.status = 'Available')
             OR r.purpose = 'Swap'
           )`,
        [this.identity]
      );
      return BigInt(rows[0].balance);
    } catch (error) {
      throw new TreeStoreError(
        `Failed to get available balance: ${error.message}`,
        error
      );
    }
  }

  async getVerifiedLeafKeys() {
    try {
      // Project just the two pubkeys out of the JSON, skipping each leaf's
      // `data` blob (up to five transactions). The filter matches the verified
      // categories the SDK expects: every reserved leaf plus every Available
      // one, and nothing non-Available and unreserved.
      const [rows] = await this.pool.query(
        `SELECT l.id AS id,
                l.verifying_public_key AS verifying,
                l.signing_public_key AS keyshare
         FROM brz_tree_leaves l
         LEFT JOIN brz_tree_reservations r
           ON l.reservation_id = r.id AND l.user_id = r.user_id
         WHERE l.user_id = ?
           AND l.is_deleted = 0
           AND (r.purpose IS NOT NULL OR l.status = 'Available')`,
        [this.identity]
      );
      return rows.map((row) => [row.id, row.verifying, row.keyshare]);
    } catch (error) {
      throw new TreeStoreError(
        `Failed to get verified leaf keys: ${error.message}`,
        error
      );
    }
  }

  async getDeletedLeaves() {
    try {
      const [rows] = await this.pool.query(
        "SELECT data FROM brz_tree_leaves WHERE user_id = ? AND is_deleted = 1",
        [this.identity]
      );
      return rows.map((row) =>
        typeof row.data === "string" ? JSON.parse(row.data) : row.data
      );
    } catch (error) {
      throw new TreeStoreError(`Failed to get deleted leaves: ${error.message}`);
    }
  }

  async removeLeaves(leafIds) {
    try {
      if (!leafIds || leafIds.length === 0) return;
      await this._withWriteTransaction(async (conn) => {
        // Each leaf owns its chain, so its ancestor rows go with it, and in
        // that order so no ancestor row is ever left without its leaf.
        // Only a row still marked and still unreserved goes: the purge read its
        // list, then spent seconds asking the operators, and a refresh landing in
        // that window may have brought the leaf back or a payment reserved it.
        for (const id of leafIds) {
          const [res] = await conn.query(
            `DELETE FROM brz_tree_leaves WHERE user_id = ? AND id = ?
               AND is_deleted = 1 AND reservation_id IS NULL`,
            [this.identity, id]
          );
          if (res.affectedRows > 0) {
            await conn.query(
              "DELETE FROM brz_tree_ancestors WHERE user_id = ? AND leaf_id = ?",
              [this.identity, id]
            );
          }
        }
      });
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(`Failed to remove leaves: ${error.message}`);
    }
  }

  async getLeaves() {
    try {
      const [rows] = await this.pool.query(
        `SELECT l.id, l.status, l.is_missing_from_operators, l.data,
                l.reservation_id, r.purpose
         FROM brz_tree_leaves l
         LEFT JOIN brz_tree_reservations r
           ON l.reservation_id = r.id AND l.user_id = r.user_id
         WHERE l.user_id = ? AND l.is_deleted = 0`,
        [this.identity]
      );

      const available = [];
      const notAvailable = [];
      const availableMissingFromOperators = [];
      const reservedForPayment = [];
      const reservedForSwap = [];

      for (const row of rows) {
        const node = parseJson(row.data);
        const spendable = node.status === "Available";

        if (row.purpose) {
          if (row.purpose === "Payment") {
            reservedForPayment.push(node);
          } else if (row.purpose === "Swap") {
            reservedForSwap.push(node);
          }
        } else if (!spendable) {
          notAvailable.push(node);
        } else if (toBool(row.is_missing_from_operators)) {
          availableMissingFromOperators.push(node);
        } else {
          available.push(node);
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
            (SELECT EXISTS(SELECT 1 FROM brz_tree_reservations WHERE user_id = ? AND purpose = 'Swap')) AS has_active_swap,
            COALESCE(
              (SELECT (last_completed_at >= ?) FROM brz_tree_swap_status WHERE user_id = ?),
              0
            ) AS swap_completed_during_refresh`,
          [this.identity, refreshTimestamp, this.identity]
        );
        const hasActiveSwap = !!swapRows[0].has_active_swap;
        const swapCompletedDuringRefresh = !!swapRows[0].swap_completed_during_refresh;

        if (hasActiveSwap || swapCompletedDuringRefresh) {
          return;
        }

        await this._cleanupSpentMarkers(conn, refreshTimestamp);

        const [spentRows] = await conn.query(
          "SELECT leaf_id FROM brz_tree_spent_leaves WHERE user_id = ? AND spent_at >= ?",
          [this.identity, refreshTimestamp]
        );
        const spentIds = new Set(spentRows.map((r) => r.leaf_id));

        // Mark, rather than remove, the non-reserved leaves added before this
        // refresh started. A leaf no operator reports may still be ours, and its
        // stored transactions are the only way to exit it, so the row stays, and
        // its ancestor rows stay with it. The upserts below clear the mark on
        // whatever came back.
        await conn.query(
          "UPDATE brz_tree_leaves SET is_deleted = 1 WHERE user_id = ? AND reservation_id IS NULL AND added_at < ? AND is_deleted = 0",
          [this.identity, refreshTimestamp]
        );

        // A leaf we spent ourselves is the one absence already accounted for, so
        // it goes for good and takes its ancestor rows with it.
        // Per id, so the ids whose rows actually went are the ones whose chains
        // go too: a spent leaf still held by a reservation keeps its row, and a
        // row without its chain is the one thing this store must never produce.
        const deletedIds = [];
        for (const id of spentIds) {
          const [res] = await conn.query(
            `DELETE FROM brz_tree_leaves WHERE user_id = ? AND reservation_id IS NULL
               AND id = ?`,
            [this.identity, id]
          );
          if (res.affectedRows > 0) deletedIds.push(id);
        }

        await this._batchUpsertLeaves(conn, leaves, false, spentIds);
        await this._batchUpsertLeaves(conn, missingLeaves, true, spentIds);

        // A leaf reported again in this same refresh is re-inserted above, so its
        // ancestor rows must survive: only ids that do NOT reappear (truly gone,
        // e.g. spent) get their ancestor rows dropped alongside them.
        const survivingIds = new Set();
        for (const leaf of leaves.concat(missingLeaves || [])) {
          if (!spentIds.has(leaf.id)) survivingIds.add(leaf.id);
        }
        const goneIds = deletedIds.filter((id) => !survivingIds.has(id));
        if (goneIds.length > 0) {
          const placeholders = buildPlaceholders(goneIds.length);
          await conn.query(
            `DELETE FROM brz_tree_ancestors WHERE user_id = ? AND leaf_id IN (${placeholders})`,
            [this.identity, ...goneIds]
          );
        }
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
        // Return leavesToKeep to the pool even when the reservation is already
        // gone (e.g. released by stale cleanup): dropping them here would lose
        // the leaves until the next refresh. The deletes no-op in that case.
        // A leaf the verification would not vouch for is marked, not dropped:
        // one operator declining to confirm it is not proof it was spent, and
        // its chain is the only way to exit it if it is still ours. The upsert
        // below clears the mark on everything kept.
        await conn.query(
          "UPDATE brz_tree_leaves SET reservation_id = NULL, is_deleted = 1 WHERE user_id = ? AND reservation_id = ?",
          [this.identity, id]
        );
        await conn.query(
          "DELETE FROM brz_tree_reservations WHERE user_id = ? AND id = ?",
          [this.identity, id]
        );

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
      // _withWriteTransaction acquires the GET_LOCK so this serializes
      // against `setLeaves`. Without it, a concurrent setLeaves could read
      // brz_tree_spent_leaves before our marker commits and re-insert the
      // just-spent leaf as Available.
      await this._withWriteTransaction(async (conn) => {
        const [resRows] = await conn.query(
          "SELECT id, purpose FROM brz_tree_reservations WHERE user_id = ? AND id = ?",
          [this.identity, id]
        );

        let isSwap = false;
        let reservedLeafIds = [];
        if (resRows.length > 0) {
          isSwap = resRows[0].purpose === "Swap";
          const [leafRows] = await conn.query(
            "SELECT id FROM brz_tree_leaves WHERE user_id = ? AND reservation_id = ?",
            [this.identity, id]
          );
          reservedLeafIds = leafRows.map((r) => r.id);
          await this._batchInsertSpentLeaves(conn, reservedLeafIds);
          await conn.query(
            "DELETE FROM brz_tree_leaves WHERE user_id = ? AND reservation_id = ?",
            [this.identity, id]
          );
          await conn.query(
            "DELETE FROM brz_tree_reservations WHERE user_id = ? AND id = ?",
            [this.identity, id]
          );
          // The spent leaves own these ancestor rows; remove them in the same
          // transaction rather than leaving them to a separate reclaim pass.
          if (reservedLeafIds.length > 0) {
            const placeholders = buildPlaceholders(reservedLeafIds.length);
            await conn.query(
              `DELETE FROM brz_tree_ancestors WHERE user_id = ? AND leaf_id IN (${placeholders})`,
              [this.identity, ...reservedLeafIds]
            );
          }
        }

        if (newLeaves && newLeaves.length > 0) {
          await this._batchUpsertLeaves(conn, newLeaves, false, null);
        }

        // UPSERT so a tenant that joined after the multi-tenant migration
        // (and thus has no row) gets one created lazily.
        if (isSwap && newLeaves && newLeaves.length > 0) {
          await conn.query(
            `INSERT INTO brz_tree_swap_status (user_id, last_completed_at) VALUES (?, UTC_TIMESTAMP(6))
             ON DUPLICATE KEY UPDATE last_completed_at = VALUES(last_completed_at)`,
            [this.identity]
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

        const [totalRows] = await conn.query(
          `SELECT COALESCE(SUM(value), 0) AS total
           FROM brz_tree_leaves
           WHERE user_id = ?
             AND status = 'Available'
             AND is_missing_from_operators = 0
             AND is_deleted = 0
             AND reservation_id IS NULL`,
          [this.identity]
        );
        const available = Number(totalRows[0].total);

        const [slimRows] = await conn.query(SLIM_LEAF_CANDIDATES_SQL, [
          this.identity,
          maxTarget,
          this.identity,
          maxTarget,
        ]);

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

  async trySelectLeaves(targetAmounts) {
    try {
      const targetAmount = targetAmounts ? this._totalSats(targetAmounts) : 0;
      const maxTarget = this._maxTargetForPrefilter(targetAmounts);

      return await this._withTransaction(async (conn) => {
        const [slimRows] = await conn.query(SLIM_LEAF_CANDIDATES_SQL, [
          this.identity,
          maxTarget,
          this.identity,
          maxTarget,
        ]);

        const slimLeaves = slimRows.map((r) => ({
          id: r.id,
          value: Number(r.value),
        }));

        const selected = this._selectLeavesByTargetAmounts(slimLeaves, targetAmounts);
        if (selected !== null && selected.length > 0) {
          const fullLeaves = await this._fetchFullLeavesByIds(
            conn,
            selected.map((l) => l.id)
          );
          return { type: "exact", leaves: fullLeaves };
        }

        const minSelected = this._selectLeavesByMinimumAmount(slimLeaves, targetAmount);
        if (minSelected !== null) {
          const fullLeaves = await this._fetchFullLeavesByIds(
            conn,
            minSelected.map((l) => l.id)
          );
          return { type: "swapNeeded", leaves: fullLeaves };
        }

        return { type: "insufficientFunds" };
      });
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to try select leaves: ${error.message}`,
        error
      );
    }
  }

  async tryReserveLeavesByIds(leafIds, purpose) {
    try {
      return await this._withWriteTransaction(async (conn) => {
        if (!leafIds || leafIds.length === 0) {
          throw new TreeStoreError("NonReservableLeaves");
        }
        // Every requested leaf must be available and unreserved; otherwise
        // reserve nothing (the transaction rolls back).
        const placeholders = leafIds.map(() => "?").join(", ");
        const [availableRows] = await conn.query(
          `SELECT id FROM brz_tree_leaves
           WHERE user_id = ? AND id IN (${placeholders})
             AND status = 'Available'
             AND is_missing_from_operators = 0
             AND is_deleted = 0
             AND reservation_id IS NULL`,
          [this.identity, ...leafIds]
        );
        if (availableRows.length !== leafIds.length) {
          throw new TreeStoreError("NonReservableLeaves");
        }
        const fullLeaves = await this._fetchFullLeavesByIds(conn, leafIds);
        const reservationId = this._generateId();
        await this._createReservation(conn, reservationId, fullLeaves, purpose, 0);
        return { id: reservationId, leaves: fullLeaves };
      });
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to try reserve leaves by ids: ${error.message}`,
        error
      );
    }
  }

  async now() {
    try {
      const [rows] = await this.pool.query("SELECT UTC_TIMESTAMP(6) AS now");
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
          "SELECT id FROM brz_tree_reservations WHERE user_id = ? AND id = ?",
          [this.identity, reservationId]
        );

        if (existsRows.length === 0) {
          throw new TreeStoreError(`Reservation ${reservationId} not found`);
        }

        const [oldLeafRows] = await conn.query(
          "SELECT id FROM brz_tree_leaves WHERE user_id = ? AND reservation_id = ?",
          [this.identity, reservationId]
        );
        const oldLeafIds = oldLeafRows.map((r) => r.id);

        await this._batchInsertSpentLeaves(conn, oldLeafIds);
        await conn.query(
          "DELETE FROM brz_tree_leaves WHERE user_id = ? AND reservation_id = ?",
          [this.identity, reservationId]
        );
        // The spent leaves own these ancestor rows; remove them now since there
        // is no later reclaim pass.
        if (oldLeafIds.length > 0) {
          const placeholders = buildPlaceholders(oldLeafIds.length);
          await conn.query(
            `DELETE FROM brz_tree_ancestors WHERE user_id = ? AND leaf_id IN (${placeholders})`,
            [this.identity, ...oldLeafIds]
          );
        }

        await this._batchUpsertLeaves(conn, changeLeaves, false, null);
        await this._batchUpsertLeaves(conn, reservedLeaves, false, null);

        const reservedLeafIds = reservedLeaves.map((l) => l.id);
        await this._batchSetReservationId(conn, reservationId, reservedLeafIds);

        await conn.query(
          "UPDATE brz_tree_reservations SET pending_change_amount = 0 WHERE user_id = ? AND id = ?",
          [this.identity, reservationId]
        );

        // Return value must be plain TreeNodes: the Rust side deserializes
        // Vec<TreeNode>.
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

  _maxTargetForPrefilter(targetAmounts) {
    if (!targetAmounts) return Number.MAX_SAFE_INTEGER;
    if (targetAmounts.type === "amountAndFee") {
      return targetAmounts.amountSats + (targetAmounts.feeSats || 0);
    }
    if (targetAmounts.type === "exactDenominations") {
      return targetAmounts.denominations.reduce((m, v) => m + v, 0);
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
      `SELECT id, data FROM brz_tree_leaves WHERE user_id = ? AND id IN (${placeholders})`,
      [this.identity, ...ids]
    );
    const byId = new Map(rows.map((r) => [r.id, r.data]));
    const ordered = ids
      .map((id) => {
        const data = byId.get(id);
        byId.delete(id);
        return data;
      })
      .filter((data) => data !== undefined)
      .map((data) => parseJson(data));
    if (ordered.length !== ids.length) {
      throw new TreeStoreError(
        `Could not resolve full data for all selected leaves (wanted ${ids.length}, got ${ordered.length})`
      );
    }
    return ordered;
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
      "SELECT COALESCE(SUM(pending_change_amount), 0) AS pending FROM brz_tree_reservations WHERE user_id = ?",
      [this.identity]
    );
    return Number(rows[0].pending);
  }

  async _createReservation(conn, reservationId, leaves, purpose, pendingChange) {
    await conn.query(
      "INSERT INTO brz_tree_reservations (user_id, id, purpose, pending_change_amount, created_at) VALUES (?, ?, ?, ?, UTC_TIMESTAMP(6))",
      [this.identity, reservationId, purpose, pendingChange]
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

    const leafNodes = filtered;

    // All chunks run inside the caller's transaction, so the full set still
    // lands atomically. UTC_TIMESTAMP(6) is re-evaluated per statement, so
    // added_at can differ by microseconds between chunks; every value is still
    // after the caller's refreshStartedAt, which is all the timestamp-based
    // deletion in setLeaves depends on.
    for (let i = 0; i < leafNodes.length; i += LEAF_UPSERT_CHUNK_SIZE) {
      const chunk = leafNodes.slice(i, i + LEAF_UPSERT_CHUNK_SIZE);
      const valueClauses = new Array(chunk.length)
        .fill("(?, ?, ?, ?, ?, ?, ?, ?, ?, UTC_TIMESTAMP(6), 0)")
        .join(", ");
      const params = [];
      for (const leaf of chunk) {
        params.push(
          this.identity,
          leaf.id,
          leaf.status,
          isMissingFromOperators ? 1 : 0,
          JSON.stringify(leaf),
          leaf.value,
          leaf.parent_node_id ?? null,
          leaf.verifying_public_key,
          leaf.signing_keyshare.public_key
        );
      }

      await conn.query(
        `INSERT INTO brz_tree_leaves
             (user_id, id, status, is_missing_from_operators, data, value,
              parent_node_id, verifying_public_key, signing_public_key, added_at,
              is_deleted)
         VALUES ${valueClauses}
         ON DUPLICATE KEY UPDATE
           status = VALUES(status),
           is_missing_from_operators = VALUES(is_missing_from_operators),
           data = VALUES(data),
           value = VALUES(value),
           parent_node_id = VALUES(parent_node_id),
           verifying_public_key = VALUES(verifying_public_key),
           signing_public_key = VALUES(signing_public_key),
           added_at = UTC_TIMESTAMP(6),
           is_deleted = 0`,
        params
      );
    }
  }

  /**
   * Replaces the ancestor rows of every pedigree wholesale (delete then
   * insert), in one delete and an insert per ANCESTOR_INSERT_CHUNK_SIZE rows. A
   * pedigree carrying no ancestors is skipped: an empty list means the chain is
   * unknown, not that the leaf has none, so a stored chain must survive being
   * re-added without one.
   */
  async _batchUpsertAncestors(conn, pedigrees) {
    const withAncestors = (pedigrees || []).filter(
      (p) => p.ancestors && p.ancestors.length > 0
    );
    if (withAncestors.length === 0) return;

    const leafIds = withAncestors.map((p) => p.leaf.id);
    await conn.query(
      `DELETE FROM brz_tree_ancestors
       WHERE user_id = ? AND leaf_id IN (${buildPlaceholders(leafIds.length)})`,
      [this.identity, ...leafIds]
    );

    const rows = [];
    for (const pedigree of withAncestors) {
      for (const node of pedigree.ancestors) {
        rows.push([
          this.identity,
          pedigree.leaf.id,
          node.id,
          node.parent_node_id ?? null,
          node.status,
          JSON.stringify(node),
          node.value,
          node.verifying_public_key,
        ]);
      }
    }

    for (let i = 0; i < rows.length; i += ANCESTOR_INSERT_CHUNK_SIZE) {
      const chunk = rows.slice(i, i + ANCESTOR_INSERT_CHUNK_SIZE);
      const valueClauses = new Array(chunk.length)
        .fill("(?, ?, ?, ?, ?, ?, ?, ?)")
        .join(", ");
      await conn.query(
        `INSERT INTO brz_tree_ancestors
             (user_id, leaf_id, id, parent_node_id, status, data, value, verifying_public_key)
         VALUES ${valueClauses}`,
        chunk.flat()
      );
    }
  }

  async _batchSetReservationId(conn, reservationId, leafIds) {
    if (leafIds.length === 0) return;

    const placeholders = buildPlaceholders(leafIds.length);
    await conn.query(
      `UPDATE brz_tree_leaves SET reservation_id = ? WHERE user_id = ? AND id IN (${placeholders})`,
      [reservationId, this.identity, ...leafIds]
    );
  }

  async _batchInsertSpentLeaves(conn, leafIds) {
    if (leafIds.length === 0) return;

    const valueClauses = new Array(leafIds.length)
      .fill("(?, ?, UTC_TIMESTAMP(6))")
      .join(", ");
    const params = [];
    for (const id of leafIds) {
      params.push(this.identity, id);
    }
    // Suppress duplicate-PK errors only — unlike INSERT IGNORE, real
    // problems (FK violations, NOT NULL violations, type errors) still
    // propagate.
    await conn.query(
      `INSERT INTO brz_tree_spent_leaves (user_id, leaf_id, spent_at) VALUES ${valueClauses}
       ON DUPLICATE KEY UPDATE leaf_id = leaf_id`,
      params
    );
  }

  async _batchRemoveSpentLeaves(conn, leafIds) {
    if (leafIds.length === 0) return;

    const placeholders = buildPlaceholders(leafIds.length);
    await conn.query(
      `DELETE FROM brz_tree_spent_leaves WHERE user_id = ? AND leaf_id IN (${placeholders})`,
      [this.identity, ...leafIds]
    );
  }

  /// Cleans up stale reservations for THIS tenant. Releases dependent leaves
  /// by clearing reservation_id first, then deletes the parent rows — the
  /// composite FK uses NO ACTION because column-list SET NULL would null
  /// user_id (NOT NULL).
  async _cleanupStaleReservations(conn) {
    await conn.query(
      `UPDATE brz_tree_leaves SET reservation_id = NULL
       WHERE user_id = ?
         AND reservation_id IN (
           SELECT id FROM (
             SELECT id FROM brz_tree_reservations
             WHERE user_id = ?
               AND created_at < DATE_SUB(UTC_TIMESTAMP(6), INTERVAL ? SECOND)
           ) AS stale
         )`,
      [this.identity, this.identity, RESERVATION_TIMEOUT_SECS]
    );
    await conn.query(
      `DELETE FROM brz_tree_reservations
       WHERE user_id = ? AND created_at < DATE_SUB(UTC_TIMESTAMP(6), INTERVAL ? SECOND)`,
      [this.identity, RESERVATION_TIMEOUT_SECS]
    );
  }

  async _cleanupSpentMarkers(conn, refreshTimestamp) {
    const cleanupCutoff = new Date(
      refreshTimestamp.getTime() - SPENT_MARKER_CLEANUP_THRESHOLD_MS
    );

    await conn.query(
      "DELETE FROM brz_tree_spent_leaves WHERE user_id = ? AND spent_at < ?",
      [this.identity, cleanupCutoff]
    );
  }
}

/**
 * Translates the `ssl-mode` URL parameter into mysql2's `ssl` option.
 * mysql2 does not understand `ssl-mode`, so it is stripped from the URI here.
 *
 * Spellings mirror the Rust SDK:
 * - absent or `disabled`: no TLS
 * - `preferred` / `required` / `verify_identity`: TLS with certificate chain
 *   and hostname verification
 * - `verify_ca`: chain verification only
 * - `no-verify`: TLS without certificate verification (explicit opt-in)
 *
 * An unrecognized value throws rather than silently downgrading to plaintext.
 * Pin a private CA with `ssl-ca=<path>` (required for `verify_ca`); for the
 * hostname-verified modes, adding the CA to Node's trust store
 * (e.g. NODE_EXTRA_CA_CERTS) also works, though it extends the store rather
 * than pinning.
 */
function extractSslMode(connectionString) {
  const queryIndex = connectionString.indexOf("?");
  if (queryIndex === -1) {
    return { connectionString, ssl: undefined };
  }
  const base = connectionString.slice(0, queryIndex);
  const retained = [];
  let sslMode;
  let sslCaPath;
  for (const param of connectionString.slice(queryIndex + 1).split("&")) {
    const eq = param.indexOf("=");
    const key = (eq === -1 ? param : param.slice(0, eq)).toLowerCase();
    if (
      eq !== -1 &&
      (key === "ssl-mode" || key === "ssl_mode" || key === "sslmode")
    ) {
      sslMode = param.slice(eq + 1).toLowerCase();
    } else if (
      eq !== -1 &&
      (key === "ssl-ca" || key === "ssl_ca" || key === "sslca")
    ) {
      sslCaPath = decodeURIComponent(param.slice(eq + 1));
    } else {
      retained.push(param);
    }
  }
  if (sslMode === undefined && sslCaPath === undefined) {
    return { connectionString, ssl: undefined };
  }
  const rebuilt = retained.length ? `${base}?${retained.join("&")}` : base;
  if (sslMode === undefined) {
    // ssl-ca alone is stripped (mysql2 would reject it as an unknown option)
    // but has no effect without an ssl-mode.
    return { connectionString: rebuilt, ssl: undefined };
  }
  const ca =
    sslCaPath === undefined
      ? undefined
      : require("fs").readFileSync(sslCaPath).toString();
  return { connectionString: rebuilt, ssl: sslOptionForMode(sslMode, ca) };
}

function sslOptionForMode(mode, ca) {
  switch (mode) {
    case "disabled":
    case "disable":
      return undefined;
    case "preferred":
    case "prefer":
    case "required":
    case "require":
    case "verify_identity":
    case "verify-identity":
    case "verifyidentity":
    case "verify-full":
    case "verify_full":
      // mysql2 checks the hostname only when verifyIdentity is set;
      // rejectUnauthorized alone verifies just the chain.
      return {
        rejectUnauthorized: true,
        verifyIdentity: true,
        ...(ca !== undefined && { ca }),
      };
    case "verify_ca":
    case "verify-ca":
    case "verifyca":
      // Without a pinned CA, chain-only verification accepts a certificate
      // from any trusted CA for any host, which authenticates nothing.
      if (ca === undefined) {
        throw new TreeStoreError(
          "ssl-mode=verify_ca requires ssl-ca=<path>; supply the CA to pin, " +
            "or use ssl-mode=required / verify_identity for " +
            "hostname-verified TLS"
        );
      }
      return { rejectUnauthorized: true, ca };
    case "no-verify":
    case "no_verify":
    case "noverify":
      return { rejectUnauthorized: false };
    default:
      throw new TreeStoreError(
        `Unrecognized ssl-mode value \`${mode}\`; expected one of: ` +
          "disabled, preferred, required, verify_ca, verify_identity, no-verify"
      );
  }
}

/** Create a mysql2 pool from a config object. */
function createMysqlPool(config) {
  const { connectionString, ssl } = extractSslMode(config.connectionString);
  return mysql.createPool({
    uri: connectionString,
    ...(ssl !== undefined && { ssl }),
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

async function createMysqlTreeStore(config, identity, logger = null) {
  const pool = createMysqlPool(config);
  return createMysqlTreeStoreWithPool(
    pool,
    identity,
    config.foreignKeyMode || "Enforced",
    logger,
    config.runMigration !== false
  );
}

async function createMysqlTreeStoreWithPool(
  pool,
  identity,
  foreignKeyMode = "Enforced",
  logger = null,
  runMigration = true
) {
  const store = new MysqlTreeStore(
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
  MysqlTreeStore,
  createMysqlTreeStore,
  createMysqlTreeStoreWithPool,
  TreeStoreError,
};
