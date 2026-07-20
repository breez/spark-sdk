/**
 * CommonJS implementation for Node.js PostgreSQL Tree Store
 */

let pg;
try {
  const mainModule = require.main;
  if (mainModule) {
    pg = mainModule.require("pg");
  } else {
    pg = require("pg");
  }
} catch (error) {
  try {
    pg = require("pg");
  } catch (fallbackError) {
    throw new Error(
      `pg not found. Please install it in your project: npm install pg@^8.18.0\n` +
        `Original error: ${error.message}\nFallback error: ${fallbackError.message}`
    );
  }
}

const { TreeStoreError } = require("./errors.cjs");
const { TreeStoreMigrationManager } = require("./migrations.cjs");

/**
 * Domain prefix mixed into the per-tenant advisory-lock key. Distinct prefixes
 * guarantee that locks from different stores (tree, token, …) never collide.
 */
const TREE_STORE_LOCK_PREFIX = "breez-spark-sdk:tree:";

/**
 * Timeout for reservations in seconds. Reservations older than this are stale.
 */
const RESERVATION_TIMEOUT_SECS = 300; // 5 minutes

/**
 * Threshold in milliseconds for cleaning up spent leaf markers.
 */
const SPENT_MARKER_CLEANUP_THRESHOLD_MS = 5 * 60 * 1000; // 5 minutes

/**
 * Slim projection: only (id, value) for leaves the selection might use.
 * Includes all leaves with value <= $2 (covers exact-match + the small-leaf
 * accumulators for the minimum-amount path) plus the single smallest leaf
 * with value > $2 (covers the minimum-amount fallback case where one larger
 * leaf is sufficient). $1 is the user id.
 */
const SLIM_LEAF_CANDIDATES_SQL = `
  SELECT id, value
  FROM brz_tree_leaves
  WHERE user_id = $1
    AND status = 'Available'
    AND is_missing_from_operators = FALSE
    AND reservation_id IS NULL
    AND (
      value <= $2
      OR id = (
        SELECT id FROM brz_tree_leaves
        WHERE user_id = $1
          AND status = 'Available'
          AND is_missing_from_operators = FALSE
          AND reservation_id IS NULL
          AND value > $2
        ORDER BY value
        LIMIT 1
      )
    )
`;

/**
 * Derive a stable per-tenant 64-bit advisory-lock key by hashing a domain
 * prefix together with the identity pubkey and folding the first 8 bytes of
 * the SHA-256 digest into a signed big-endian i64 — the type expected by
 * `pg_advisory_xact_lock(bigint)`. The 64-bit space keeps cross-tenant
 * collisions negligible (~1.2e-10 at 65k tenants).
 */
function _identityLockKey(prefix, identity) {
  const crypto = require("crypto");
  const hash = crypto.createHash("sha256");
  hash.update(prefix);
  hash.update(Buffer.from(identity));
  return hash.digest().readBigInt64BE(0);
}

class PostgresTreeStore {
  /**
   * @param {import('pg').Pool} pool
   * @param {Buffer|Uint8Array} identity - 33-byte secp256k1 compressed pubkey
   *   identifying the tenant. All reads and writes are scoped by this.
   * @param {object} [logger]
   */
  constructor(pool, identity, logger = null, runMigration = true) {
    if (!identity || identity.length !== 33) {
      throw new TreeStoreError(
        "tenant identity (33-byte secp256k1 pubkey) is required"
      );
    }
    this.pool = pool;
    this.identity = Buffer.from(identity);
    this.lockKey = _identityLockKey(TREE_STORE_LOCK_PREFIX, identity);
    this.logger = logger;
    this.runMigration = runMigration;
  }

  /**
   * Initialize the database (run migrations)
   */
  async initialize() {
    try {
      if (this.runMigration) {
        const migrationManager = new TreeStoreMigrationManager(this.logger);
        await migrationManager.migrate(this.pool, this.identity);
      }
      return this;
    } catch (error) {
      throw new TreeStoreError(
        `Failed to initialize PostgreSQL tree store: ${error.message}`,
        error
      );
    }
  }

  /**
   * Close the pool
   */
  async close() {
    if (this.pool) {
      await this.pool.end();
      this.pool = null;
    }
  }

  /**
   * Run a function inside a transaction with the advisory lock. Used by every
   * write whose correctness depends on serializing against the GC and the
   * available-leaf set: `tryReserveLeaves`, `setLeaves`, `finalizeReservation`,
   * `addLeaves`, `cancelReservation`, `updateReservation`.
   * @param {function(import('pg').PoolClient): Promise<T>} fn
   * @returns {Promise<T>}
   * @template T
   */
  async _withWriteTransaction(fn) {
    const client = await this.pool.connect();
    try {
      await client.query("BEGIN");
      // Per-tenant advisory lock: 64-bit key derived from a tree-store domain
      // prefix and the tenant identity, so different tenants don't serialize
      // on each other and tree/token locks never collide.
      await client.query("SELECT pg_advisory_xact_lock($1)", [this.lockKey]);
      const result = await fn(client);
      await client.query("COMMIT");
      return result;
    } catch (error) {
      await client.query("ROLLBACK").catch(() => {});
      throw error;
    } finally {
      client.release();
    }
  }

  /**
   * Run a function inside a transaction without the advisory lock. Used only by
   * read-only previews (`trySelectLeaves`) that take no locks and mutate nothing.
   * @param {function(import('pg').PoolClient): Promise<T>} fn
   * @returns {Promise<T>}
   * @template T
   */
  async _withTransaction(fn) {
    const client = await this.pool.connect();
    try {
      await client.query("BEGIN");
      const result = await fn(client);
      await client.query("COMMIT");
      return result;
    } catch (error) {
      await client.query("ROLLBACK").catch(() => {});
      throw error;
    } finally {
      client.release();
    }
  }

  // ===== TreeStore Methods =====

  /**
   * Add leaves to the store. Removes from spent leaves first, then upserts.
   * @param {Array} leaves - Array of TreeNode objects
   */
  async addLeaves(leaves) {
    try {
      if (!leaves || leaves.length === 0) {
        return;
      }

      const leafNodes = leaves.map((p) => p.leaf);
      await this._withWriteTransaction(async (client) => {
        // Remove these leaves from spent_leaves table
        const leafIds = leafNodes.map((l) => l.id);
        await this._batchRemoveSpentLeaves(client, leafIds);

        for (const pedigree of leaves) {
          await this._batchUpsertAncestors(client, pedigree.ancestors);
        }
        // Batch upsert all leaves
        await this._batchUpsertLeaves(client, leaves, false, null);
      });
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to add leaves: ${error.message}`,
        error
      );
    }
  }

  async getNode(id) {
    try {
      const result = await this.pool.query(
        `SELECT data FROM brz_tree_leaves WHERE user_id = $1 AND id = $2
         UNION ALL
         SELECT data FROM brz_tree_ancestors WHERE user_id = $1 AND id = $2
         LIMIT 1`,
        [this.identity, id]
      );
      return result.rows.length > 0 ? result.rows[0].data : null;
    } catch (error) {
      throw new TreeStoreError(`Failed to get node '${id}': ${error.message}`, error);
    }
  }

  async getNodes(ids) {
    try {
      if (!ids || ids.length === 0) return [];
      const result = await this.pool.query(
        `SELECT data FROM brz_tree_leaves WHERE user_id = $1 AND id = ANY($2)
         UNION ALL
         SELECT a.data FROM brz_tree_ancestors a
         WHERE a.user_id = $1 AND a.id = ANY($2)
           AND NOT EXISTS (
             SELECT 1 FROM brz_tree_leaves l
             WHERE l.user_id = $1 AND l.id = a.id
           )`,
        [this.identity, ids]
      );
      return result.rows.map((r) => r.data);
    } catch (error) {
      throw new TreeStoreError(`Failed to get nodes: ${error.message}`, error);
    }
  }

  async getExitChain(leafId) {
    try {
      const result = await this.pool.query(
        `WITH RECURSIVE nodes(id, parent_node_id, data) AS (
             SELECT id, parent_node_id, data
             FROM brz_tree_leaves WHERE user_id = $1
             UNION ALL
             SELECT a.id, a.parent_node_id, a.data
             FROM brz_tree_ancestors a
             WHERE a.user_id = $1
               AND NOT EXISTS (
                 SELECT 1 FROM brz_tree_leaves l
                 WHERE l.user_id = $1 AND l.id = a.id
               )
         ),
         chain(id, parent_node_id, data, depth) AS (
             SELECT id, parent_node_id, data, 0 FROM nodes WHERE id = $2
             UNION ALL
             SELECT n.id, n.parent_node_id, n.data, c.depth + 1
             FROM nodes n JOIN chain c ON n.id = c.parent_node_id
             WHERE c.depth < 1000
         )
         SELECT data FROM chain ORDER BY depth DESC`,
        [this.identity, leafId]
      );
      const chain = result.rows.map((r) => r.data);
      if (chain.length === 0) {
        console.warn(
          `exit chain for leaf ${leafId} is incomplete: leaf not found in store`
        );
      } else if (chain[0].parent_node_id != null) {
        console.warn(
          `exit chain for leaf ${leafId} is incomplete: an ancestor is missing from store`
        );
      }
      return chain;
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to get exit chain for '${leafId}': ${error.message}`,
        error
      );
    }
  }

  /**
   * Get all leaves categorized by status.
   * @returns {Promise<Object>} Leaves object with available, notAvailable, etc.
   */
  /**
   * Returns the wallet's spendable balance (available + missing-from-operators
   * + swap-reserved). Aggregated server-side so we don't fetch every leaf.
   * @returns {Promise<bigint>}
   */
  async getAvailableBalance() {
    try {
      const result = await this.pool.query(
        `
        SELECT COALESCE(SUM(l.value), 0)::bigint AS balance
        FROM brz_tree_leaves l
        LEFT JOIN brz_tree_reservations r
          ON l.reservation_id = r.id AND l.user_id = r.user_id
        WHERE l.user_id = $1
          AND (
            (l.reservation_id IS NULL AND l.status = 'Available')
            OR r.purpose = 'Swap'
          )
      `,
        [this.identity]
      );
      return BigInt(result.rows[0].balance);
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
      const result = await this.pool.query(
        `
        SELECT l.id AS id,
               l.verifying_public_key AS verifying,
               l.signing_public_key AS keyshare
        FROM brz_tree_leaves l
        LEFT JOIN brz_tree_reservations r
          ON l.reservation_id = r.id AND l.user_id = r.user_id
        WHERE l.user_id = $1
          AND (r.purpose IS NOT NULL OR l.status = 'Available')
      `,
        [this.identity]
      );
      return result.rows.map((row) => [row.id, row.verifying, row.keyshare]);
    } catch (error) {
      throw new TreeStoreError(
        `Failed to get verified leaf keys: ${error.message}`,
        error
      );
    }
  }

  async getLeaves() {
    try {
      const result = await this.pool.query(
        `
        SELECT l.id, l.status, l.is_missing_from_operators, l.data,
               l.reservation_id, r.purpose
        FROM brz_tree_leaves l
        LEFT JOIN brz_tree_reservations r
          ON l.reservation_id = r.id AND l.user_id = r.user_id
        WHERE l.user_id = $1
      `,
        [this.identity]
      );

      const available = [];
      const notAvailable = [];
      const availableMissingFromOperators = [];
      const reservedForPayment = [];
      const reservedForSwap = [];

      for (const row of result.rows) {
        const node = row.data;
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
      await this._withWriteTransaction(async (client) => {
        const refreshTimestamp = new Date(refreshStartedAtMs);

        // Drop expired reservations BEFORE evaluating has_active_swap, otherwise a stale
        // Swap reservation (from a crashed client or a swap whose finalize/cancel never
        // ran) keeps has_active_swap true forever, which makes set_leaves early-return
        // and never reach the cleanup again. The reservation pins itself in place.
        await this._cleanupStaleReservations(client);

        // Check for active swap or swap completed during refresh
        const swapCheckResult = await client.query(
          `
          SELECT
            EXISTS(
              SELECT 1 FROM brz_tree_reservations
              WHERE user_id = $1 AND purpose = 'Swap'
            ) AS has_active_swap,
            COALESCE(
              (SELECT last_completed_at >= $2
               FROM brz_tree_swap_status WHERE user_id = $1),
              FALSE
            ) AS swap_completed_during_refresh
        `,
          [this.identity, refreshTimestamp]
        );

        const { has_active_swap, swap_completed_during_refresh } = swapCheckResult.rows[0];

        if (has_active_swap || swap_completed_during_refresh) {
          return;
        }

        // Clean up old spent markers
        await this._cleanupSpentMarkers(client, refreshTimestamp);

        const spentResult = await client.query(
          "SELECT leaf_id FROM brz_tree_spent_leaves WHERE user_id = $1 AND spent_at >= $2",
          [this.identity, refreshTimestamp]
        );
        const spentIds = new Set(spentResult.rows.map((r) => r.leaf_id));

        // Delete non-reserved leaves added before refresh started.
        // Includes leaves released earlier in this transaction by
        // _cleanupStaleReservations (which now NULLs reservation_id explicitly,
        // since the composite FK uses NO ACTION).
        await client.query(
          "DELETE FROM brz_tree_leaves WHERE user_id = $1 AND reservation_id IS NULL AND added_at < $2",
          [this.identity, refreshTimestamp]
        );

        for (const pedigree of leaves.concat(missingLeaves || [])) {
          await this._batchUpsertAncestors(client, pedigree.ancestors);
        }
        // Upsert all leaves (filtering spent)
        await this._batchUpsertLeaves(client, leaves, false, spentIds);
        await this._batchUpsertLeaves(client, missingLeaves, true, spentIds);
        await this._gcAncestors(client);
      });
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to set leaves: ${error.message}`,
        error
      );
    }
  }

  /**
   * Cancel a reservation. All leaves currently attached to the reservation are
   * deleted from the store. The reservation row is dropped. The supplied
   * `leavesToKeep` are inserted into the available pool.
   *
   * Callers pass the original reservation leaves to preserve the legacy
   * "release everything back to the pool" behavior. Callers that have
   * verified leaf state with the operator pass only the leaves confirmed
   * safe to make available (e.g. dropping operator-locked leaves).
   *
   * @param {string} id - Reservation ID
   * @param {Array} leavesToKeep - Leaves to insert as available
   */
  async cancelReservation(id, leavesToKeep) {
    try {
      await this._withWriteTransaction(async (client) => {
        // Return leavesToKeep to the pool even when the reservation is already
        // gone (e.g. released by stale cleanup): dropping them here would lose
        // the leaves until the next refresh. The deletes no-op in that case.
        await client.query(
          "DELETE FROM brz_tree_leaves WHERE user_id = $1 AND reservation_id = $2",
          [this.identity, id]
        );

        await client.query(
          "DELETE FROM brz_tree_reservations WHERE user_id = $1 AND id = $2",
          [this.identity, id]
        );

        if (leavesToKeep && leavesToKeep.length > 0) {
          for (const pedigree of leavesToKeep) {
            await this._batchUpsertAncestors(client, pedigree.ancestors);
          }
          await this._batchUpsertLeaves(client, leavesToKeep, false, null);
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

  /**
   * Finalize a reservation, marking leaves as spent.
   * @param {string} id - Reservation ID
   * @param {Array|null} newLeaves - Optional new leaves to add
   */
  async finalizeReservation(id, newLeaves) {
    try {
      // _withWriteTransaction acquires the advisory lock so this serializes
      // against `setLeaves`. Without it, a concurrent setLeaves could read
      // brz_tree_spent_leaves before our marker commits and re-insert the
      // just-spent leaf as Available.
      await this._withWriteTransaction(async (client) => {
        // Check if reservation exists and get purpose
        const res = await client.query(
          "SELECT id, purpose FROM brz_tree_reservations WHERE user_id = $1 AND id = $2",
          [this.identity, id]
        );

        let isSwap = false;
        let reservedLeafIds = [];
        if (res.rows.length > 0) {
          isSwap = res.rows[0].purpose === "Swap";
          const leafResult = await client.query(
            "SELECT id FROM brz_tree_leaves WHERE user_id = $1 AND reservation_id = $2",
            [this.identity, id]
          );
          reservedLeafIds = leafResult.rows.map((r) => r.id);
          await this._batchInsertSpentLeaves(client, reservedLeafIds);
          await client.query(
            "DELETE FROM brz_tree_leaves WHERE user_id = $1 AND reservation_id = $2",
            [this.identity, id]
          );
          await client.query(
            "DELETE FROM brz_tree_reservations WHERE user_id = $1 AND id = $2",
            [this.identity, id]
          );
        }

        // Add new leaves if provided
        if (newLeaves && newLeaves.length > 0) {
          for (const pedigree of newLeaves) {
            await this._batchUpsertAncestors(client, pedigree.ancestors);
          }
          await this._batchUpsertLeaves(client, newLeaves, false, null);
        }
        await this._gcAncestors(client);

        // If swap with new leaves, update last_completed_at. UPSERT so a tenant
        // that joined after migration 3 (and thus has no row) gets one created.
        if (isSwap && newLeaves && newLeaves.length > 0) {
          await client.query(
            `INSERT INTO brz_tree_swap_status (user_id, last_completed_at)
             VALUES ($1, NOW())
             ON CONFLICT (user_id) DO UPDATE
               SET last_completed_at = EXCLUDED.last_completed_at`,
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

  /**
   * Try to reserve leaves matching target amounts.
   * @param {Object|null} targetAmounts - Target amounts spec
   * @param {boolean} exactOnly - If true, only exact matches
   * @param {string} purpose - "Payment" or "Swap"
   * @returns {Promise<Object>} ReserveResult
   */
  async tryReserveLeaves(targetAmounts, exactOnly, purpose) {
    try {
      return await this._withWriteTransaction(async (client) => {
        const targetAmount = targetAmounts ? this._totalSats(targetAmounts) : 0;
        const maxTarget = this._maxTargetForPrefilter(targetAmounts);

        // True total available, computed server-side over ALL eligible leaves.
        // Required for the WaitForPending decision below — must NOT be derived
        // from the prefiltered set since the prefilter may exclude big leaves.
        const totalResult = await client.query(
          `
          SELECT COALESCE(SUM(value), 0)::bigint AS total
          FROM brz_tree_leaves
          WHERE user_id = $1
            AND status = 'Available'
            AND is_missing_from_operators = FALSE
            AND reservation_id IS NULL
        `,
          [this.identity]
        );
        const available = Number(totalResult.rows[0].total);

        const slimResult = await client.query(SLIM_LEAF_CANDIDATES_SQL, [
          this.identity,
          maxTarget,
        ]);

        const slimLeaves = slimResult.rows.map((r) => ({
          id: r.id,
          value: Number(r.value),
        }));

        // Calculate pending balance
        const pending = await this._calculatePendingBalance(client);

        // Try exact selection on slim leaves — selection only reads .id/.value
        const selected = this._selectLeavesByTargetAmounts(slimLeaves, targetAmounts);

        if (selected !== null) {
          if (selected.length === 0) {
            throw new TreeStoreError("NonReservableLeaves");
          }

          const fullLeaves = await this._fetchFullLeavesByIds(
            client,
            selected.map((l) => l.id)
          );
          const reservationId = this._generateId();
          await this._createReservation(client, reservationId, fullLeaves, purpose, 0);

          return {
            type: "success",
            reservation: {
              id: reservationId,
              leaves: fullLeaves,
            },
          };
        }

        if (!exactOnly) {
          // Try minimum amount selection on the slim set
          const minSelected = this._selectLeavesByMinimumAmount(slimLeaves, targetAmount);
          if (minSelected !== null) {
            const fullLeaves = await this._fetchFullLeavesByIds(
              client,
              minSelected.map((l) => l.id)
            );
            const reservedAmount = fullLeaves.reduce((sum, l) => sum + l.value, 0);
            const pendingChange = reservedAmount > targetAmount && targetAmount > 0
              ? reservedAmount - targetAmount
              : 0;

            const reservationId = this._generateId();
            await this._createReservation(client, reservationId, fullLeaves, purpose, pendingChange);

            return {
              type: "success",
              reservation: {
                id: reservationId,
                leaves: fullLeaves,
              },
            };
          }
        }

        // No suitable leaves found
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

      return await this._withTransaction(async (client) => {
        const slimResult = await client.query(SLIM_LEAF_CANDIDATES_SQL, [
          this.identity,
          maxTarget,
        ]);

        const slimLeaves = slimResult.rows.map((r) => ({
          id: r.id,
          value: Number(r.value),
        }));

        const selected = this._selectLeavesByTargetAmounts(slimLeaves, targetAmounts);
        if (selected !== null && selected.length > 0) {
          const fullLeaves = await this._fetchFullLeavesByIds(
            client,
            selected.map((l) => l.id)
          );
          return { type: "exact", leaves: fullLeaves };
        }

        const minSelected = this._selectLeavesByMinimumAmount(slimLeaves, targetAmount);
        if (minSelected !== null) {
          const fullLeaves = await this._fetchFullLeavesByIds(
            client,
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
      return await this._withWriteTransaction(async (client) => {
        if (!leafIds || leafIds.length === 0) {
          throw new TreeStoreError("NonReservableLeaves");
        }
        // Every requested leaf must be available and unreserved; otherwise
        // reserve nothing (the transaction rolls back).
        const availableResult = await client.query(
          `
          SELECT id FROM brz_tree_leaves
          WHERE user_id = $1
            AND id = ANY($2)
            AND status = 'Available'
            AND is_missing_from_operators = FALSE
            AND reservation_id IS NULL
        `,
          [this.identity, leafIds]
        );
        if (availableResult.rows.length !== leafIds.length) {
          throw new TreeStoreError("NonReservableLeaves");
        }
        const fullLeaves = await this._fetchFullLeavesByIds(client, leafIds);
        const reservationId = this._generateId();
        await this._createReservation(client, reservationId, fullLeaves, purpose, 0);
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
  async _fetchFullLeavesByIds(client, ids) {
    if (!ids || ids.length === 0) return [];
    const result = await client.query(
      "SELECT id, data FROM brz_tree_leaves WHERE user_id = $2 AND id = ANY($1)",
      [ids, this.identity]
    );
    const byId = new Map(result.rows.map((r) => [r.id, r.data]));
    const ordered = ids
      .map((id) => {
        const data = byId.get(id);
        byId.delete(id);
        return data;
      })
      .filter((data) => data !== undefined);
    if (ordered.length !== ids.length) {
      throw new TreeStoreError(
        `Could not resolve full data for all selected leaves (wanted ${ids.length}, got ${ordered.length})`
      );
    }
    return ordered;
  }

  /**
   * Get current database time as epoch milliseconds.
   * @returns {Promise<number>}
   */
  async now() {
    try {
      const result = await this.pool.query("SELECT NOW()");
      return result.rows[0].now.getTime();
    } catch (error) {
      throw new TreeStoreError(
        `Failed to get current time: ${error.message}`,
        error
      );
    }
  }

  /**
   * Update a reservation after a swap.
   * @param {string} reservationId - Existing reservation ID
   * @param {Array} reservedLeaves - New reserved leaves
   * @param {Array} changeLeaves - Change leaves to add to available pool
   * @returns {Promise<Object>} Updated reservation
   */
  async updateReservation(reservationId, reservedLeaves, changeLeaves) {
    try {
      return await this._withWriteTransaction(async (client) => {
        // Check if reservation exists
        const res = await client.query(
          "SELECT id FROM brz_tree_reservations WHERE user_id = $1 AND id = $2",
          [this.identity, reservationId]
        );

        if (res.rows.length === 0) {
          throw new TreeStoreError(`Reservation ${reservationId} not found`);
        }

        // Get old reserved leaf IDs and mark as spent
        const oldLeavesResult = await client.query(
          "SELECT id FROM brz_tree_leaves WHERE user_id = $1 AND reservation_id = $2",
          [this.identity, reservationId]
        );
        const oldLeafIds = oldLeavesResult.rows.map((r) => r.id);

        await this._batchInsertSpentLeaves(client, oldLeafIds);
        await client.query(
          "DELETE FROM brz_tree_leaves WHERE user_id = $1 AND reservation_id = $2",
          [this.identity, reservationId]
        );

        // The swap outputs carry their ancestors so they stay offline-exitable.
        for (const pedigree of changeLeaves.concat(reservedLeaves)) {
          await this._batchUpsertAncestors(client, pedigree.ancestors);
        }
        // Upsert change leaves to available pool
        await this._batchUpsertLeaves(client, changeLeaves, false, null);

        // Upsert reserved leaves
        await this._batchUpsertLeaves(client, reservedLeaves, false, null);

        // Set reservation_id on reserved leaves
        const reservedLeafIds = reservedLeaves.map((p) => p.leaf.id);
        await this._batchSetReservationId(client, reservationId, reservedLeafIds);

        // Clear pending change amount
        await client.query(
          "UPDATE brz_tree_reservations SET pending_change_amount = 0 WHERE user_id = $1 AND id = $2",
          [this.identity, reservationId]
        );

        // Return value must be plain TreeNodes: the Rust side deserializes
        // Vec<TreeNode>.
        return {
          id: reservationId,
          leaves: reservedLeaves.map((p) => p.leaf),
        };
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

  /**
   * Generate a unique reservation ID (UUIDv4).
   */
  _generateId() {
    // Use crypto.randomUUID if available, otherwise manual
    if (typeof crypto !== "undefined" && crypto.randomUUID) {
      return crypto.randomUUID();
    }
    // Fallback UUIDv4
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

  /**
   * Calculate pending balance from in-flight swaps.
   */
  async _calculatePendingBalance(client) {
    const result = await client.query(
      "SELECT COALESCE(SUM(pending_change_amount), 0)::BIGINT AS pending FROM brz_tree_reservations WHERE user_id = $1",
      [this.identity]
    );
    return Number(result.rows[0].pending);
  }

  /**
   * Create a reservation with the given leaves.
   */
  async _createReservation(client, reservationId, leaves, purpose, pendingChange) {
    await client.query(
      "INSERT INTO brz_tree_reservations (user_id, id, purpose, pending_change_amount) VALUES ($1, $2, $3, $4)",
      [this.identity, reservationId, purpose, pendingChange]
    );

    const leafIds = leaves.map((l) => l.id);
    await this._batchSetReservationId(client, reservationId, leafIds);
  }

  /**
   * Batch upsert leaves into brz_tree_leaves table.
   */
  async _batchUpsertLeaves(client, pedigrees, isMissingFromOperators, skipIds) {
    if (!pedigrees || pedigrees.length === 0) return;

    const filtered = skipIds
      ? pedigrees.filter((p) => !skipIds.has(p.leaf.id))
      : pedigrees;

    if (filtered.length === 0) return;

    const leafNodes = filtered.map((p) => p.leaf);
    await this._checkNodesCompatible(client, leafNodes);

    const ids = leafNodes.map((l) => l.id);
    const statuses = leafNodes.map((l) => l.status);
    const missingFlags = leafNodes.map(() => isMissingFromOperators);
    const dataValues = leafNodes.map((l) => JSON.stringify(l));
    const values = leafNodes.map((l) => l.value);
    const parents = leafNodes.map((l) => l.parent_node_id ?? null);
    const verifyings = leafNodes.map((l) => l.verifying_public_key);
    const signings = leafNodes.map((l) => l.signing_keyshare.public_key);

    await client.query(
      `INSERT INTO brz_tree_leaves
           (user_id, id, status, is_missing_from_operators, data, added_at,
            value, parent_node_id, verifying_public_key, signing_public_key)
       SELECT $5, id, status, missing, data::jsonb, NOW(),
              value, parent_node_id, verifying, signing
       FROM UNNEST($1::text[], $2::text[], $3::bool[], $4::text[],
                   $6::bigint[], $7::text[], $8::text[], $9::text[])
           AS t(id, status, missing, data, value, parent_node_id, verifying, signing)
       ON CONFLICT (user_id, id) DO UPDATE SET
         status = EXCLUDED.status,
         is_missing_from_operators = EXCLUDED.is_missing_from_operators,
         data = EXCLUDED.data,
         added_at = NOW(),
         value = EXCLUDED.value,
         parent_node_id = EXCLUDED.parent_node_id,
         verifying_public_key = EXCLUDED.verifying_public_key,
         signing_public_key = EXCLUDED.signing_public_key`,
      [
        ids,
        statuses,
        missingFlags,
        dataValues,
        this.identity,
        values,
        parents,
        verifyings,
        signings,
      ]
    );
  }

  /**
   * Errors if any incoming node conflicts with a stored node of the same id on
   * a field that must not change (value, verifying key). One query loads the
   * existing rows (leaf or ancestor) for the batch.
   */
  async _checkNodesCompatible(client, nodes) {
    if (!nodes || nodes.length === 0) return;
    const ids = nodes.map((n) => n.id);
    const result = await client.query(
      `SELECT id, value, verifying_public_key
         FROM brz_tree_leaves WHERE user_id = $1 AND id = ANY($2)
       UNION ALL
       SELECT id, value, verifying_public_key
         FROM brz_tree_ancestors WHERE user_id = $1 AND id = ANY($2)`,
      [this.identity, ids]
    );
    const existing = new Map();
    for (const row of result.rows) {
      existing.set(row.id, row);
    }
    for (const node of nodes) {
      const old = existing.get(node.id);
      if (!old) continue;
      // The `value` column is BIGINT (returned as a string by pg); coerce both
      // sides to strings so a number-vs-string mismatch is not a false positive.
      if (String(old.value) !== String(node.value)) {
        throw new TreeStoreError(
          `node ${node.id} value changed from ${old.value} to ${node.value}`
        );
      }
      if (old.verifying_public_key !== node.verifying_public_key) {
        throw new TreeStoreError(`node ${node.id} verifying public key changed`);
      }
    }
  }

  /**
   * Batch upserts ancestors. Mutable fields (status, parent, data) are
   * refreshed on conflict.
   */
  async _batchUpsertAncestors(client, nodes) {
    if (!nodes || nodes.length === 0) return;
    await this._checkNodesCompatible(client, nodes);

    const ids = nodes.map((n) => n.id);
    const parents = nodes.map((n) => n.parent_node_id ?? null);
    const statuses = nodes.map((n) => n.status);
    const dataValues = nodes.map((n) => JSON.stringify(n));
    const values = nodes.map((n) => n.value);
    const verifyings = nodes.map((n) => n.verifying_public_key);

    await client.query(
      `INSERT INTO brz_tree_ancestors
           (user_id, id, parent_node_id, status, data, value, verifying_public_key)
       SELECT $5, id, parent_node_id, status, data::jsonb, value, verifying
       FROM UNNEST($1::text[], $2::text[], $3::text[], $4::text[],
                   $6::bigint[], $7::text[])
           AS t(id, parent_node_id, status, data, value, verifying)
       ON CONFLICT (user_id, id) DO UPDATE SET
         parent_node_id = EXCLUDED.parent_node_id,
         status = EXCLUDED.status,
         data = EXCLUDED.data`,
      [ids, parents, statuses, dataValues, this.identity, values, verifyings]
    );
  }

  /**
   * Deletes ancestors no longer on any leaf's parent chain (a deleted leaf's
   * unshared ancestors); ancestors still shared by a surviving leaf are kept.
   */
  async _gcAncestors(client) {
    await client.query(
      `WITH RECURSIVE reachable(id) AS (
           SELECT parent_node_id FROM brz_tree_leaves
           WHERE user_id = $1 AND parent_node_id IS NOT NULL
           UNION
           SELECT a.parent_node_id FROM brz_tree_ancestors a
           JOIN reachable r ON a.id = r.id
           WHERE a.user_id = $1 AND a.parent_node_id IS NOT NULL
       )
       DELETE FROM brz_tree_ancestors
       WHERE user_id = $1 AND id NOT IN (SELECT id FROM reachable)`,
      [this.identity]
    );
  }

  /**
   * Batch set reservation_id on leaves.
   */
  async _batchSetReservationId(client, reservationId, leafIds) {
    if (leafIds.length === 0) return;

    await client.query(
      "UPDATE brz_tree_leaves SET reservation_id = $1 WHERE user_id = $3 AND id = ANY($2)",
      [reservationId, leafIds, this.identity]
    );
  }

  /**
   * Batch insert spent leaf markers.
   */
  async _batchInsertSpentLeaves(client, leafIds) {
    if (leafIds.length === 0) return;

    await client.query(
      `INSERT INTO brz_tree_spent_leaves (user_id, leaf_id)
       SELECT $2, leaf_id FROM UNNEST($1::text[]) AS t(leaf_id)
       ON CONFLICT DO NOTHING`,
      [leafIds, this.identity]
    );
  }

  /**
   * Batch remove spent leaf markers.
   */
  async _batchRemoveSpentLeaves(client, leafIds) {
    if (leafIds.length === 0) return;

    await client.query(
      "DELETE FROM brz_tree_spent_leaves WHERE user_id = $2 AND leaf_id = ANY($1)",
      [leafIds, this.identity]
    );
  }

  /**
   * Clean up stale reservations. Releases the leaves by clearing their
   * reservation_id first, then deletes the parent reservations — the composite
   * FK uses NO ACTION (the default) since column-list SET NULL is PG15+ and a
   * whole-row SET NULL would null user_id (NOT NULL).
   */
  async _cleanupStaleReservations(client) {
    await client.query(
      `UPDATE brz_tree_leaves SET reservation_id = NULL
       WHERE user_id = $2
         AND reservation_id IN (
           SELECT id FROM brz_tree_reservations
           WHERE user_id = $2
             AND created_at < NOW() - make_interval(secs => $1)
         )`,
      [RESERVATION_TIMEOUT_SECS, this.identity]
    );
    await client.query(
      `DELETE FROM brz_tree_reservations
       WHERE user_id = $2
         AND created_at < NOW() - make_interval(secs => $1)`,
      [RESERVATION_TIMEOUT_SECS, this.identity]
    );
  }

  /**
   * Clean up old spent markers.
   */
  async _cleanupSpentMarkers(client, refreshTimestamp) {
    const thresholdMs = SPENT_MARKER_CLEANUP_THRESHOLD_MS;
    const cleanupCutoff = new Date(refreshTimestamp.getTime() - thresholdMs);

    await client.query(
      "DELETE FROM brz_tree_spent_leaves WHERE user_id = $2 AND spent_at < $1",
      [cleanupCutoff, this.identity]
    );
  }
}

/**
 * Create a PostgresTreeStore instance from a config object.
 *
 * @param {object} config - PostgreSQL configuration
 * @param {string} config.connectionString - PostgreSQL connection string
 * @param {number} config.maxPoolSize - Maximum number of connections in the pool
 * @param {number} config.createTimeoutSecs - Timeout in seconds for establishing a new connection
 * @param {number} config.recycleTimeoutSecs - Timeout in seconds before recycling an idle connection
 * @param {Buffer|Uint8Array} identity - 33-byte secp256k1 compressed pubkey scoping reads/writes
 * @param {object} [logger] - Optional logger
 * @returns {Promise<PostgresTreeStore>}
 */
async function createPostgresTreeStore(config, identity, logger = null) {
  const pool = new pg.Pool({
    connectionString: config.connectionString,
    max: config.maxPoolSize,
    connectionTimeoutMillis: config.createTimeoutSecs * 1000,
    idleTimeoutMillis: config.recycleTimeoutSecs * 1000,
  });
  return createPostgresTreeStoreWithPool(
    pool,
    identity,
    logger,
    config.runMigration !== false
  );
}

/**
 * Create a PostgresTreeStore instance from an existing pg.Pool.
 *
 * @param {pg.Pool} pool - An existing connection pool
 * @param {Buffer|Uint8Array} identity - 33-byte secp256k1 compressed pubkey scoping reads/writes
 * @param {object} [logger] - Optional logger
 * @returns {Promise<PostgresTreeStore>}
 */
async function createPostgresTreeStoreWithPool(
  pool,
  identity,
  logger = null,
  runMigration = true
) {
  const store = new PostgresTreeStore(
    pool,
    identity,
    logger,
    runMigration
  );
  await store.initialize();
  return store;
}

module.exports = { PostgresTreeStore, createPostgresTreeStore, createPostgresTreeStoreWithPool, TreeStoreError };
