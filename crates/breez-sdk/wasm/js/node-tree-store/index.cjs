/**
 * CommonJS implementation for the Node.js SQLite Tree Store.
 *
 * The single durable source of truth for one wallet's leaves, ancestors,
 * reservations, and spent records. It shares the wallet's main better-sqlite3
 * database file, so its tables are `brz_`-prefixed to stay clear of the main
 * storage's. There is no tenant/user column and no advisory locking:
 * better-sqlite3 is synchronous and each method runs its transaction to
 * completion without yielding.
 *
 * Two-table model (see migrations.cjs): `brz_tree_leaves` is the spendable pool;
 * `brz_tree_ancestors` holds the intermediate exit-chain nodes. SQL mirrors the
 * Rust `spark-sqlite` store (crates/spark-sqlite/src/lib.rs). Selection logic
 * mirrors the PostgreSQL tree store (js/postgres-tree-store/index.cjs).
 */

// Resolve better-sqlite3 from the calling module's context, same as node-storage.
let Database;
try {
  const mainModule = require.main;
  if (mainModule) {
    Database = mainModule.require("better-sqlite3");
  } else {
    Database = require("better-sqlite3");
  }
} catch (error) {
  try {
    Database = require("better-sqlite3");
  } catch (fallbackError) {
    throw new Error(
      `better-sqlite3 not found. Please install it in your project: npm install better-sqlite3@^9.2.2\n` +
        `Original error: ${error.message}\nFallback error: ${fallbackError.message}`
    );
  }
}

const { TreeStoreError } = require("./errors.cjs");
const { TreeStoreMigrationManager } = require("./migrations.cjs");

/**
 * Reservations idle longer than this are treated as abandoned by a crashed
 * client and released during setLeaves.
 */
const RESERVATION_TIMEOUT_MS = 5 * 60 * 1000; // 5 minutes

/**
 * Spent markers older than this (relative to a refresh) are pruned.
 */
const SPENT_MARKER_CLEANUP_THRESHOLD_MS = 5 * 60 * 1000; // 5 minutes

/**
 * Slim projection: only (id, value) for leaves the selection might use. Every
 * eligible leaf with value <= ? plus the single smallest eligible leaf above it
 * (the minimum-amount fallback where one larger leaf suffices). The bound is
 * bound twice positionally.
 */
const SLIM_LEAF_CANDIDATES_SQL = `
  SELECT id, value FROM brz_tree_leaves
  WHERE status = 'Available'
    AND is_missing_from_operators = 0
    AND reservation_id IS NULL
    AND (
      value <= ?
      OR id = (
        SELECT id FROM brz_tree_leaves
        WHERE status = 'Available'
          AND is_missing_from_operators = 0
          AND reservation_id IS NULL
          AND value > ?
        ORDER BY value
        LIMIT 1
      )
    )
`;

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

class NodeTreeStore {
  /**
   * @param {string} dbPath - Path to the SQLite database file for this wallet.
   * @param {object} [logger]
   * @param {boolean} [runMigration]
   */
  constructor(dbPath, logger = null, runMigration = true) {
    this.dbPath = dbPath;
    this.db = null;
    this.migrationManager = null;
    this.logger = logger;
    this.runMigration = runMigration;
  }

  /**
   * Open the database and run migrations. Returns the store instance.
   */
  initialize() {
    try {
      this.db = new Database(this.dbPath);
      // Shared file: WAL lets this connection read/write alongside the main
      // storage's, and the busy timeout waits for the lock instead of failing.
      this.db.pragma("journal_mode = WAL");
      this.db.pragma("busy_timeout = 5000");
      if (this.runMigration) {
        this.migrationManager = new TreeStoreMigrationManager(
          this.db,
          TreeStoreError,
          this.logger
        );
        this.migrationManager.migrate();
      }
      return this;
    } catch (error) {
      throw new TreeStoreError(
        `Failed to initialize tree store at '${this.dbPath}': ${error.message}`,
        error
      );
    }
  }

  /**
   * Close the database connection.
   */
  close() {
    if (this.db) {
      this.db.close();
      this.db = null;
    }
  }

  // ===== TreeStore Methods =====

  /**
   * Add leaves to the pool together with their ancestors. Receiving a leaf back
   * clears any prior spent marker. Re-adding an id refreshes its mutable fields.
   * @param {Array} leaves - Array of LeafPedigree { leaf, ancestors }
   */
  async addLeaves(leaves) {
    try {
      if (!leaves || leaves.length === 0) {
        return;
      }
      const leafNodes = leaves.map((p) => p.leaf);
      this.db.transaction(() => {
        this._removeSpent(leafNodes.map((l) => l.id));
        for (const pedigree of leaves) {
          this._upsertAncestors(pedigree.ancestors);
        }
        this._upsertLeaves(leafNodes, false, null);
      })();
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(`Failed to add leaves: ${error.message}`, error);
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
      // Load the requested leaves plus every ancestor (the ancestor table only
      // holds this wallet's chains), then walk in memory. Ancestors come first so
      // a leaf overwrites an ancestor of the same id.
      const placeholders = leafIds.map(() => "?").join(",");
      const rows = this.db
        .prepare(
          `SELECT data FROM brz_tree_ancestors
           UNION ALL
           SELECT data FROM brz_tree_leaves WHERE id IN (${placeholders})`
        )
        .all(...leafIds);
      const nodes = new Map();
      for (const r of rows) {
        const node = JSON.parse(r.data);
        nodes.set(node.id, node);
      }
      return leafIds
        .map((id) => assembleExitChain(nodes, id))
        .filter((p) => p != null);
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(`Failed to get exit chains: ${error.message}`, error);
    }
  }

  /**
   * Return the wallet's spendable balance (available + swap-reserved) as a
   * BigInt. Aggregated in SQL so we don't fetch every leaf.
   * @returns {Promise<bigint>}
   */
  async getAvailableBalance() {
    try {
      const row = this.db
        .prepare(
          `SELECT COALESCE(SUM(l.value), 0) AS balance
           FROM brz_tree_leaves l
           LEFT JOIN brz_tree_reservations r ON l.reservation_id = r.id
           WHERE (l.reservation_id IS NULL AND l.status = 'Available')
              OR r.purpose = 'Swap'`
        )
        .get();
      return BigInt(row.balance);
    } catch (error) {
      throw new TreeStoreError(
        `Failed to get available balance: ${error.message}`,
        error
      );
    }
  }

  /**
   * Return [id, verifyingPublicKey, signingKeysharePublicKey] triples for every
   * reserved or Available leaf. The two pubkeys are projected out of the JSON so
   * we skip each leaf's transaction blob.
   * @returns {Promise<Array<[string, string, string]>>}
   */
  async getVerifiedLeafKeys() {
    try {
      const rows = this.db
        .prepare(
          `SELECT l.id AS id,
                  l.verifying_public_key AS verifying,
                  l.signing_public_key AS keyshare
           FROM brz_tree_leaves l
           LEFT JOIN brz_tree_reservations r ON l.reservation_id = r.id
           WHERE r.purpose IS NOT NULL OR l.status = 'Available'`
        )
        .all();
      return rows.map((row) => [row.id, row.verifying, row.keyshare]);
    } catch (error) {
      throw new TreeStoreError(
        `Failed to get verified leaf keys: ${error.message}`,
        error
      );
    }
  }

  /**
   * Return all pool leaves categorized by status and reservation purpose.
   * @returns {Promise<Object>}
   */
  async getLeaves() {
    try {
      const rows = this.db
        .prepare(
          `SELECT l.status, l.is_missing_from_operators, l.data,
                  l.reservation_id, r.purpose
           FROM brz_tree_leaves l
           LEFT JOIN brz_tree_reservations r ON l.reservation_id = r.id`
        )
        .all();

      const available = [];
      const notAvailable = [];
      const availableMissingFromOperators = [];
      const reservedForPayment = [];
      const reservedForSwap = [];

      for (const row of rows) {
        const node = JSON.parse(row.data);
        const spendable = node.status === "Available";

        if (row.purpose) {
          if (row.purpose === "Payment") {
            reservedForPayment.push(node);
          } else if (row.purpose === "Swap") {
            reservedForSwap.push(node);
          }
        } else if (!spendable) {
          notAvailable.push(node);
        } else if (row.is_missing_from_operators) {
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
      throw new TreeStoreError(`Failed to get leaves: ${error.message}`, error);
    }
  }

  /**
   * Replace the pool from a refresh. Skipped while a swap is in flight or one
   * completed during the refresh, so a swap's leaves are never clobbered.
   * @param {Array} leaves - Available LeafPedigree { leaf, ancestors }
   * @param {Array} missingLeaves - LeafPedigree { leaf, ancestors } missing from some operators
   * @param {number} refreshStartedAtMs - Epoch milliseconds when refresh started
   */
  async setLeaves(leaves, missingLeaves, refreshStartedAtMs) {
    try {
      const refreshMs = refreshStartedAtMs;
      this.db.transaction(() => {
        // Release abandoned reservations before evaluating the swap guard so a
        // stale swap cannot pin setLeaves forever.
        this._cleanupStaleReservations();

        const hasActiveSwap =
          this.db
            .prepare(
              "SELECT EXISTS(SELECT 1 FROM brz_tree_reservations WHERE purpose = 'Swap') AS has_active_swap"
            )
            .get().has_active_swap !== 0;
        const statusRow = this.db
          .prepare("SELECT last_completed_at FROM brz_tree_swap_status WHERE id = 1")
          .get();
        const swapCompleted =
          statusRow != null &&
          statusRow.last_completed_at != null &&
          statusRow.last_completed_at >= refreshMs;

        if (hasActiveSwap || swapCompleted) {
          return;
        }

        this._cleanupSpentMarkers(refreshMs);
        const spentIds = this._spentIdsSince(refreshMs);

        // Delete non-reserved pool leaves older than the refresh; reserved and
        // after-refresh leaves are immune. Leaves present in the refresh below
        // are re-inserted, and their now-unshared ancestors collected afterwards.
        const deleted = this.db
          .prepare(
            "DELETE FROM brz_tree_leaves WHERE reservation_id IS NULL AND added_at < ?"
          )
          .run(refreshMs).changes;

        for (const pedigree of leaves) {
          this._upsertAncestors(pedigree.ancestors);
        }
        for (const pedigree of missingLeaves) {
          this._upsertAncestors(pedigree.ancestors);
        }
        this._upsertLeaves(leaves.map((p) => p.leaf), false, spentIds);
        this._upsertLeaves(missingLeaves.map((p) => p.leaf), true, spentIds);
        // Only a deleted leaf can orphan an ancestor; skip the walk otherwise.
        if (deleted > 0) {
          this._gcAncestors();
        }
      })();
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(`Failed to set leaves: ${error.message}`, error);
    }
  }

  /**
   * Cancel a reservation. Its leaves are deleted from the pool and the row is
   * dropped. `leavesToKeep` are re-inserted into the available pool; their
   * ancestors are already stored (they stayed while the leaves were reserved).
   * @param {string} id
   * @param {Array} leavesToKeep - TreeNode leaves to return to the available pool
   */
  async cancelReservation(id, leavesToKeep) {
    try {
      this.db.transaction(() => {
        // Return leavesToKeep to the pool even when the reservation is already
        // gone (e.g. released by stale cleanup): dropping them here would lose
        // the leaves until the next refresh. The deletes no-op in that case.
        // Only the leaves are re-inserted: their ancestors stayed in the ancestor
        // table the whole time they were reserved.
        this.db.prepare("DELETE FROM brz_tree_leaves WHERE reservation_id = ?").run(id);
        this.db.prepare("DELETE FROM brz_tree_reservations WHERE id = ?").run(id);
        this._upsertLeaves(leavesToKeep, false, null);
      })();
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to cancel reservation '${id}': ${error.message}`,
        error
      );
    }
  }

  /**
   * Finalize a reservation, marking its leaves spent and adding any new leaves.
   * @param {string} id
   * @param {Array|null} newLeaves - Optional new LeafPedigree { leaf, ancestors }
   */
  async finalizeReservation(id, newLeaves) {
    try {
      this.db.transaction(() => {
        const res = this.db
          .prepare("SELECT id, purpose FROM brz_tree_reservations WHERE id = ?")
          .get(id);

        let isSwap = false;
        let deleted = 0;
        if (res) {
          isSwap = res.purpose === "Swap";
          const reservedLeafIds = this.db
            .prepare("SELECT id FROM brz_tree_leaves WHERE reservation_id = ?")
            .all(id)
            .map((r) => r.id);
          this._insertSpent(reservedLeafIds);
          deleted = this.db
            .prepare("DELETE FROM brz_tree_leaves WHERE reservation_id = ?")
            .run(id).changes;
          this.db.prepare("DELETE FROM brz_tree_reservations WHERE id = ?").run(id);
        }

        if (newLeaves && newLeaves.length > 0) {
          for (const pedigree of newLeaves) {
            this._upsertAncestors(pedigree.ancestors);
          }
          this._upsertLeaves(newLeaves.map((p) => p.leaf), false, null);
        }
        // Only a deleted (spent) leaf can orphan an ancestor; skip the walk otherwise.
        if (deleted > 0) {
          this._gcAncestors();
        }

        // Record the swap only when it produced change: the setLeaves guard uses
        // this to skip a refresh that raced the swap.
        if (isSwap && newLeaves && newLeaves.length > 0) {
          this._markSwapCompleted();
        }
      })();
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to finalize reservation '${id}': ${error.message}`,
        error
      );
    }
  }

  /**
   * Try to reserve leaves matching target amounts.
   * @param {Object|null} targetAmounts
   * @param {boolean} exactOnly - If true, only exact matches
   * @param {string} purpose - "Payment" or "Swap"
   * @returns {Promise<Object>} ReserveResult
   */
  async tryReserveLeaves(targetAmounts, exactOnly, purpose) {
    try {
      return this.db.transaction(() => {
        const targetAmount = targetAmounts ? this._totalSats(targetAmounts) : 0;
        const maxTarget = this._maxTargetForPrefilter(targetAmounts);

        // True total available over ALL eligible leaves, for the WaitForPending
        // decision below: must not be derived from the prefiltered slim set.
        const available = this._availableTotal();
        const slimLeaves = this._slimCandidates(maxTarget);
        const pending = this._pendingBalance();

        const selected = this._selectLeavesByTargetAmounts(slimLeaves, targetAmounts);
        if (selected !== null) {
          if (selected.length === 0) {
            throw new TreeStoreError("NonReservableLeaves");
          }
          const fullLeaves = this._resolveFullLeaves(selected.map((l) => l.id));
          const reservationId = this._generateId();
          this._createReservation(reservationId, fullLeaves, purpose, 0);
          return {
            type: "success",
            reservation: { id: reservationId, leaves: fullLeaves },
          };
        }

        if (!exactOnly) {
          const minSelected = this._selectLeavesByMinimumAmount(slimLeaves, targetAmount);
          if (minSelected !== null) {
            const fullLeaves = this._resolveFullLeaves(minSelected.map((l) => l.id));
            const reservedAmount = fullLeaves.reduce((sum, l) => sum + l.value, 0);
            const pendingChange =
              reservedAmount > targetAmount && targetAmount > 0
                ? reservedAmount - targetAmount
                : 0;
            const reservationId = this._generateId();
            this._createReservation(reservationId, fullLeaves, purpose, pendingChange);
            return {
              type: "success",
              reservation: { id: reservationId, leaves: fullLeaves },
            };
          }
        }

        if (available + pending >= targetAmount) {
          return { type: "waitForPending", needed: targetAmount, available, pending };
        }
        return { type: "insufficientFunds" };
      })();
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to try reserve leaves: ${error.message}`,
        error
      );
    }
  }

  /**
   * Select (without reserving) leaves matching target amounts.
   * @param {Object|null} targetAmounts
   * @returns {Promise<Object>} LeafSelection
   */
  async trySelectLeaves(targetAmounts) {
    try {
      const targetAmount = targetAmounts ? this._totalSats(targetAmounts) : 0;
      const maxTarget = this._maxTargetForPrefilter(targetAmounts);

      return this.db.transaction(() => {
        const slimLeaves = this._slimCandidates(maxTarget);

        const selected = this._selectLeavesByTargetAmounts(slimLeaves, targetAmounts);
        if (selected !== null && selected.length > 0) {
          const fullLeaves = this._resolveFullLeaves(selected.map((l) => l.id));
          return { type: "exact", leaves: fullLeaves };
        }

        const minSelected = this._selectLeavesByMinimumAmount(slimLeaves, targetAmount);
        if (minSelected !== null) {
          const fullLeaves = this._resolveFullLeaves(minSelected.map((l) => l.id));
          return { type: "swapNeeded", leaves: fullLeaves };
        }

        return { type: "insufficientFunds" };
      })();
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to try select leaves: ${error.message}`,
        error
      );
    }
  }

  /**
   * Reserve exactly the given leaves. Every id must be available and unreserved,
   * or nothing is reserved.
   * @param {Array<string>} leafIds
   * @param {string} purpose - "Payment" or "Swap"
   * @returns {Promise<Object>} { id, leaves }
   */
  async tryReserveLeavesByIds(leafIds, purpose) {
    try {
      return this.db.transaction(() => {
        if (!leafIds || leafIds.length === 0) {
          throw new TreeStoreError("NonReservableLeaves");
        }
        // Count DISTINCT matching ids so a duplicate id can't satisfy two slots:
        // every requested id must resolve to its own available, unreserved leaf.
        const placeholders = leafIds.map(() => "?").join(", ");
        const matched = this.db
          .prepare(
            `SELECT DISTINCT id FROM brz_tree_leaves
             WHERE id IN (${placeholders}) AND status = 'Available'
               AND is_missing_from_operators = 0 AND reservation_id IS NULL`
          )
          .all(...leafIds);
        if (matched.length !== leafIds.length) {
          throw new TreeStoreError("NonReservableLeaves");
        }
        const fullLeaves = this._resolveFullLeaves(leafIds);
        const reservationId = this._generateId();
        this._createReservation(reservationId, fullLeaves, purpose, 0);
        return { id: reservationId, leaves: fullLeaves };
      })();
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to try reserve leaves by ids: ${error.message}`,
        error
      );
    }
  }

  /**
   * Current wall-clock time as epoch milliseconds.
   * @returns {Promise<number>}
   */
  async now() {
    return Date.now();
  }

  /**
   * Update a reservation after a swap: spend the old reserved leaves, add the
   * change leaves to the pool, and attach the new reserved leaves.
   * @param {string} reservationId
   * @param {Array} reservedLeaves - New reserved LeafPedigree { leaf, ancestors }
   * @param {Array} changeLeaves - Change LeafPedigree { leaf, ancestors } for the available pool
   * @returns {Promise<Object>} { id, leaves }
   */
  async updateReservation(reservationId, reservedLeaves, changeLeaves) {
    try {
      return this.db.transaction(() => {
        const res = this.db
          .prepare("SELECT id FROM brz_tree_reservations WHERE id = ?")
          .get(reservationId);
        if (!res) {
          throw new TreeStoreError(`Reservation ${reservationId} not found`);
        }

        const oldLeafIds = this.db
          .prepare("SELECT id FROM brz_tree_leaves WHERE reservation_id = ?")
          .all(reservationId)
          .map((r) => r.id);
        this._insertSpent(oldLeafIds);
        this.db
          .prepare("DELETE FROM brz_tree_leaves WHERE reservation_id = ?")
          .run(reservationId);

        for (const pedigree of changeLeaves) {
          this._upsertAncestors(pedigree.ancestors);
        }
        for (const pedigree of reservedLeaves) {
          this._upsertAncestors(pedigree.ancestors);
        }
        this._upsertLeaves(changeLeaves.map((p) => p.leaf), false, null);
        this._upsertLeaves(reservedLeaves.map((p) => p.leaf), false, null);
        this._setReservationId(
          reservationId,
          reservedLeaves.map((p) => p.leaf.id)
        );

        this.db
          .prepare(
            "UPDATE brz_tree_reservations SET pending_change_amount = 0 WHERE id = ?"
          )
          .run(reservationId);

        // Return value must be plain TreeNodes: the Rust side deserializes
        // Vec<TreeNode>.
        return { id: reservationId, leaves: reservedLeaves.map((p) => p.leaf) };
      })();
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to update reservation '${reservationId}': ${error.message}`,
        error
      );
    }
  }

  // ===== Private DB helpers (synchronous) =====

  /**
   * Look a node up as a leaf first, then as an ancestor. Returns the parsed
   * node or null.
   */
  _getNodeSync(id) {
    const row = this.db
      .prepare(
        `SELECT data FROM brz_tree_leaves WHERE id = ?
         UNION ALL
         SELECT data FROM brz_tree_ancestors WHERE id = ?
         LIMIT 1`
      )
      .get(id, id);
    return row ? JSON.parse(row.data) : null;
  }

  /**
   * Error if an incoming node conflicts with a stored node of the same id on a
   * field that must not change (value, verifying key). Mirrors the Rust
   * `ensure_node_compatible` rule.
   */
  _checkCompatible(node) {
    const existing = this._getNodeSync(node.id);
    if (!existing) return;
    if (existing.value !== node.value) {
      throw new TreeStoreError(
        `node ${node.id} value changed from ${existing.value} to ${node.value}`
      );
    }
    if (existing.verifying_public_key !== node.verifying_public_key) {
      throw new TreeStoreError(`node ${node.id} verifying public key changed`);
    }
  }

  /**
   * Upsert leaf pedigrees into the pool, skipping any id in `skipIds` (spent).
   * Refreshes the leaf's mutable fields; preserves reservation_id (not in the
   * SET list).
   */
  _upsertLeaves(leaves, isMissing, skipIds) {
    if (!leaves || leaves.length === 0) return;
    const stmt = this.db.prepare(
      `INSERT INTO brz_tree_leaves
           (id, parent_node_id, status, value, verifying_public_key,
            signing_public_key, data, is_missing_from_operators, added_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT(id) DO UPDATE SET
           parent_node_id = excluded.parent_node_id,
           status = excluded.status,
           value = excluded.value,
           verifying_public_key = excluded.verifying_public_key,
           signing_public_key = excluded.signing_public_key,
           data = excluded.data,
           is_missing_from_operators = excluded.is_missing_from_operators,
           added_at = excluded.added_at`
    );
    const now = Date.now();
    for (const leaf of leaves) {
      if (skipIds && skipIds.has(leaf.id)) continue;
      this._checkCompatible(leaf);
      stmt.run(
        leaf.id,
        leaf.parent_node_id ?? null,
        leaf.status,
        leaf.value,
        leaf.verifying_public_key,
        leaf.signing_keyshare.public_key,
        JSON.stringify(leaf),
        isMissing ? 1 : 0,
        now
      );
    }
  }

  /**
   * Upsert ancestors. Mutable fields (status, parent, data) are refreshed on
   * conflict; value is immutable and left untouched.
   */
  _upsertAncestors(nodes) {
    if (!nodes || nodes.length === 0) return;
    const stmt = this.db.prepare(
      `INSERT INTO brz_tree_ancestors
           (id, parent_node_id, status, value, verifying_public_key, data)
       VALUES (?, ?, ?, ?, ?, ?)
       ON CONFLICT(id) DO UPDATE SET
           status = excluded.status,
           parent_node_id = excluded.parent_node_id,
           data = excluded.data`
    );
    for (const node of nodes) {
      this._checkCompatible(node);
      stmt.run(
        node.id,
        node.parent_node_id ?? null,
        node.status,
        node.value,
        node.verifying_public_key,
        JSON.stringify(node)
      );
    }
  }

  /**
   * Delete ancestors no longer on any leaf's parent chain; ancestors still
   * shared by a surviving leaf are kept.
   */
  _gcAncestors() {
    this.db
      .prepare(
        `WITH RECURSIVE reachable(id) AS (
             SELECT parent_node_id FROM brz_tree_leaves
             WHERE parent_node_id IS NOT NULL
             UNION
             SELECT a.parent_node_id FROM brz_tree_ancestors a
             JOIN reachable r ON a.id = r.id
             WHERE a.parent_node_id IS NOT NULL
         )
         DELETE FROM brz_tree_ancestors WHERE id NOT IN (SELECT id FROM reachable)`
      )
      .run();
  }

  /**
   * Full node data for the selected ids, preserving selection order. Errors if
   * any selected leaf is missing.
   */
  _resolveFullLeaves(ids) {
    if (!ids || ids.length === 0) return [];
    const stmt = this.db.prepare("SELECT data FROM brz_tree_leaves WHERE id = ?");
    const leaves = [];
    for (const id of ids) {
      const row = stmt.get(id);
      if (!row) {
        throw new TreeStoreError(`selected leaf ${id} not found in store`);
      }
      leaves.push(JSON.parse(row.data));
    }
    return leaves;
  }

  /**
   * Create a reservation and attach it to the given leaves.
   */
  _createReservation(id, leaves, purpose, pendingChange) {
    this.db
      .prepare(
        "INSERT INTO brz_tree_reservations (id, purpose, pending_change_amount, created_at) VALUES (?, ?, ?, ?)"
      )
      .run(id, purpose, pendingChange, Date.now());
    this._setReservationId(
      id,
      leaves.map((l) => l.id)
    );
  }

  _setReservationId(reservationId, leafIds) {
    if (!leafIds || leafIds.length === 0) return;
    const stmt = this.db.prepare(
      "UPDATE brz_tree_leaves SET reservation_id = ? WHERE id = ?"
    );
    for (const id of leafIds) {
      stmt.run(reservationId, id);
    }
  }

  _insertSpent(ids) {
    if (!ids || ids.length === 0) return;
    const stmt = this.db.prepare(
      "INSERT OR IGNORE INTO brz_tree_spent (id, spent_at) VALUES (?, ?)"
    );
    const now = Date.now();
    for (const id of ids) {
      stmt.run(id, now);
    }
  }

  _removeSpent(ids) {
    if (!ids || ids.length === 0) return;
    const stmt = this.db.prepare("DELETE FROM brz_tree_spent WHERE id = ?");
    for (const id of ids) {
      stmt.run(id);
    }
  }

  _spentIdsSince(refreshMs) {
    const rows = this.db
      .prepare("SELECT id FROM brz_tree_spent WHERE spent_at >= ?")
      .all(refreshMs);
    return new Set(rows.map((r) => r.id));
  }

  _cleanupStaleReservations() {
    const cutoff = Date.now() - RESERVATION_TIMEOUT_MS;
    this.db
      .prepare(
        `UPDATE brz_tree_leaves SET reservation_id = NULL
         WHERE reservation_id IN (
           SELECT id FROM brz_tree_reservations WHERE created_at < ?
         )`
      )
      .run(cutoff);
    this.db
      .prepare("DELETE FROM brz_tree_reservations WHERE created_at < ?")
      .run(cutoff);
  }

  _cleanupSpentMarkers(refreshMs) {
    this.db
      .prepare("DELETE FROM brz_tree_spent WHERE spent_at < ?")
      .run(refreshMs - SPENT_MARKER_CLEANUP_THRESHOLD_MS);
  }

  _markSwapCompleted() {
    this.db
      .prepare("UPDATE brz_tree_swap_status SET last_completed_at = ? WHERE id = 1")
      .run(Date.now());
  }

  /**
   * Total value of unreserved available pool leaves (drives WaitForPending).
   */
  _availableTotal() {
    return this.db
      .prepare(
        `SELECT COALESCE(SUM(value), 0) AS total FROM brz_tree_leaves
         WHERE status = 'Available'
           AND is_missing_from_operators = 0 AND reservation_id IS NULL`
      )
      .get().total;
  }

  _pendingBalance() {
    return this.db
      .prepare(
        "SELECT COALESCE(SUM(pending_change_amount), 0) AS pending FROM brz_tree_reservations"
      )
      .get().pending;
  }

  _slimCandidates(maxTarget) {
    return this.db
      .prepare(SLIM_LEAF_CANDIDATES_SQL)
      .all(maxTarget, maxTarget)
      .map((r) => ({ id: r.id, value: r.value }));
  }

  // ===== Private selection helpers (pure) =====

  /**
   * Generate a unique reservation ID (UUIDv4).
   */
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

  /**
   * Calculate total sats from target amounts.
   */
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
   * Select leaves by target amounts. Returns null if no exact match found.
   */
  _selectLeavesByTargetAmounts(leaves, targetAmounts) {
    if (!targetAmounts) {
      // No target: return all leaves (may be empty)
      return [...leaves];
    }

    if (targetAmounts.type === "amountAndFee") {
      const amountLeaves = this._selectLeavesByExactAmount(leaves, targetAmounts.amountSats);
      if (amountLeaves === null) return null;

      if (targetAmounts.feeSats != null && targetAmounts.feeSats > 0) {
        const amountIds = new Set(amountLeaves.map((l) => l.id));
        const remaining = leaves.filter((l) => !amountIds.has(l.id));
        const feeLeaves = this._selectLeavesByExactAmount(remaining, targetAmounts.feeSats);
        if (feeLeaves === null) return null;
        return [...amountLeaves, ...feeLeaves];
      }

      return amountLeaves;
    }

    if (targetAmounts.type === "exactDenominations") {
      return this._selectLeavesByExactDenominations(leaves, targetAmounts.denominations);
    }

    return null;
  }

  /**
   * Select leaves that sum to exactly the target amount.
   */
  _selectLeavesByExactAmount(leaves, targetAmount) {
    if (targetAmount === 0) return null; // Invalid amount

    const totalAvailable = leaves.reduce((sum, l) => sum + l.value, 0);
    if (totalAvailable < targetAmount) return null; // Insufficient funds

    // Try single exact match
    const single = leaves.find((l) => l.value === targetAmount);
    if (single) return [single];

    // Try greedy multiple match
    const multipleResult = this._findExactMultipleMatch(leaves, targetAmount);
    return multipleResult;
  }

  /**
   * Select leaves that match exact denominations.
   */
  _selectLeavesByExactDenominations(leaves, denominations) {
    const remaining = [...leaves];
    const selected = [];

    for (const denomination of denominations) {
      const idx = remaining.findIndex((l) => l.value === denomination);
      if (idx === -1) return null; // Can't match this denomination
      selected.push(remaining[idx]);
      remaining.splice(idx, 1);
    }

    return selected;
  }

  /**
   * Select leaves summing to at least the target amount.
   */
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

  /**
   * Find exact multiple match using greedy algorithm.
   */
  _findExactMultipleMatch(leaves, targetAmount) {
    if (targetAmount === 0) return [];
    if (leaves.length === 0) return null;

    // Pass 1: Try greedy on all leaves
    const result = this._greedyExactMatch(leaves, targetAmount);
    if (result) return result;

    // Pass 2: Try with only power-of-two leaves
    const powerOfTwoLeaves = leaves.filter((l) => this._isPowerOfTwo(l.value));
    if (powerOfTwoLeaves.length === leaves.length) return null;

    return this._greedyExactMatch(powerOfTwoLeaves, targetAmount);
  }

  /**
   * Greedy exact match algorithm.
   */
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

  /**
   * Check if value is a power of two.
   */
  _isPowerOfTwo(value) {
    return value > 0 && (value & (value - 1)) === 0;
  }
}

/**
 * Create and initialize a NodeTreeStore for the SQLite database at `dbPath`.
 * Returns the initialized store instance. Unlike the async PostgreSQL factory,
 * this is synchronous because better-sqlite3 initialization is synchronous; the
 * return value is still awaitable.
 *
 * @param {string} dbPath - Path to the SQLite database file for this wallet.
 * @param {object} [logger]
 * @param {boolean} [runMigration]
 * @returns {NodeTreeStore}
 */
// Async so it satisfies the wasm-bindgen `async fn` import the Rust bridge binds
// to, even though better-sqlite3 initialization itself is synchronous.
async function createNodeTreeStore(dbPath, logger = null, runMigration = true) {
  const store = new NodeTreeStore(dbPath, logger, runMigration);
  return store.initialize();
}

module.exports = { NodeTreeStore, createNodeTreeStore, TreeStoreError };
