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
 * Advisory lock key for serializing tree store write operations.
 * Matches the Rust constant TREE_STORE_WRITE_LOCK_KEY = 0x7472_6565_5354_4f52
 */
const TREE_STORE_WRITE_LOCK_KEY = "8391086132283252818"; // 0x7472656553544f52 as decimal string

/**
 * Timeout for reservations in seconds. Reservations older than this are stale.
 */
const RESERVATION_TIMEOUT_SECS = 300; // 5 minutes

/**
 * Threshold in milliseconds for cleaning up spent leaf markers.
 */
const SPENT_MARKER_CLEANUP_THRESHOLD_MS = 5 * 60 * 1000; // 5 minutes

class PostgresTreeStore {
  constructor(pool, logger = null) {
    this.pool = pool;
    this.logger = logger;
  }

  /**
   * Initialize the database (run migrations)
   */
  async initialize() {
    try {
      const migrationManager = new TreeStoreMigrationManager(this.logger);
      await migrationManager.migrate(this.pool);
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
   * Run a function inside a transaction with the advisory lock.
   * @param {function(import('pg').PoolClient): Promise<T>} fn
   * @returns {Promise<T>}
   * @template T
   */
  async _withWriteTransaction(fn) {
    const client = await this.pool.connect();
    try {
      await client.query("BEGIN");
      await client.query(`SELECT pg_advisory_xact_lock(${TREE_STORE_WRITE_LOCK_KEY})`);
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

      await this._withWriteTransaction(async (client) => {
        // Remove these leaves from spent_leaves table
        const leafIds = leaves.map((l) => l.id);
        await this._batchRemoveSpentLeaves(client, leafIds);

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

  /**
   * Get all leaves categorized by status.
   * @returns {Promise<Object>} Leaves object with available, notAvailable, etc.
   */
  async getLeaves() {
    try {
      const result = await this.pool.query(`
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

      for (const row of result.rows) {
        const node = row.data;

        if (row.purpose) {
          if (row.purpose === "Payment") {
            reservedForPayment.push(node);
          } else if (row.purpose === "Swap") {
            reservedForSwap.push(node);
          }
        } else if (row.is_missing_from_operators) {
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
      await this._withWriteTransaction(async (client) => {
        const refreshTimestamp = new Date(refreshStartedAtMs);

        // Check for active swap or swap completed during refresh
        const swapCheckResult = await client.query(`
          SELECT
            EXISTS(SELECT 1 FROM tree_reservations WHERE purpose = 'Swap') AS has_active_swap,
            COALESCE((SELECT last_completed_at >= $1 FROM tree_swap_status WHERE id = 1), FALSE) AS swap_completed_during_refresh
        `, [refreshTimestamp]);

        const { has_active_swap, swap_completed_during_refresh } = swapCheckResult.rows[0];

        if (has_active_swap || swap_completed_during_refresh) {
          return;
        }

        // Clean up old spent markers
        await this._cleanupSpentMarkers(client, refreshTimestamp);

        // Get recent spent leaf IDs (spent_at >= refresh_timestamp)
        const spentResult = await client.query(
          "SELECT leaf_id FROM tree_spent_leaves WHERE spent_at >= $1",
          [refreshTimestamp]
        );
        const spentIds = new Set(spentResult.rows.map((r) => r.leaf_id));

        // Delete non-reserved leaves added before refresh started
        await client.query(
          "DELETE FROM tree_leaves WHERE reservation_id IS NULL AND added_at < $1",
          [refreshTimestamp]
        );

        // Clean up stale reservations (after leaf delete)
        await this._cleanupStaleReservations(client);

        // Upsert all leaves (filtering spent)
        await this._batchUpsertLeaves(client, leaves, false, spentIds);
        await this._batchUpsertLeaves(client, missingLeaves, true, spentIds);
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
   * Cancel a reservation, releasing reserved leaves.
   * @param {string} id - Reservation ID
   */
  async cancelReservation(id) {
    try {
      await this._withWriteTransaction(async (client) => {
        // Check if reservation exists
        const res = await client.query(
          "SELECT id FROM tree_reservations WHERE id = $1",
          [id]
        );

        if (res.rows.length === 0) {
          return; // Already cancelled or finalized
        }

        // Delete reservation (ON DELETE SET NULL releases leaves)
        await client.query(
          "DELETE FROM tree_reservations WHERE id = $1",
          [id]
        );
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
      await this._withWriteTransaction(async (client) => {
        // Check if reservation exists and get purpose
        const res = await client.query(
          "SELECT id, purpose FROM tree_reservations WHERE id = $1",
          [id]
        );

        if (res.rows.length === 0) {
          return; // Already finalized or cancelled
        }

        const isSwap = res.rows[0].purpose === "Swap";

        // Get reserved leaf IDs
        const leafResult = await client.query(
          "SELECT id FROM tree_leaves WHERE reservation_id = $1",
          [id]
        );
        const reservedLeafIds = leafResult.rows.map((r) => r.id);

        // Mark as spent
        await this._batchInsertSpentLeaves(client, reservedLeafIds);

        // Delete reserved leaves and reservation
        await client.query(
          "DELETE FROM tree_leaves WHERE reservation_id = $1",
          [id]
        );
        await client.query(
          "DELETE FROM tree_reservations WHERE id = $1",
          [id]
        );

        // Add new leaves if provided
        if (newLeaves && newLeaves.length > 0) {
          await this._batchUpsertLeaves(client, newLeaves, false, null);
        }

        // If swap with new leaves, update last_completed_at
        if (isSwap && newLeaves && newLeaves.length > 0) {
          await client.query(
            "UPDATE tree_swap_status SET last_completed_at = NOW() WHERE id = 1"
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

        // Clean up expired reservations so their leaves become available again.
        await this._cleanupStaleReservations(client);

        // Get available leaves
        const availableResult = await client.query(`
          SELECT data
          FROM tree_leaves
          WHERE status = 'Available'
            AND is_missing_from_operators = FALSE
            AND reservation_id IS NULL
        `);

        const availableLeaves = availableResult.rows.map((r) => r.data);
        const available = availableLeaves.reduce((sum, l) => sum + l.value, 0);

        // Calculate pending balance
        const pending = await this._calculatePendingBalance(client);

        // Try exact selection first
        const selected = this._selectLeavesByTargetAmounts(availableLeaves, targetAmounts);

        if (selected !== null) {
          if (selected.length === 0) {
            throw new TreeStoreError("NonReservableLeaves");
          }

          const reservationId = this._generateId();
          await this._createReservation(client, reservationId, selected, purpose, 0);

          return {
            type: "success",
            reservation: {
              id: reservationId,
              leaves: selected,
            },
          };
        }

        if (!exactOnly) {
          // Try minimum amount selection
          const minSelected = this._selectLeavesByMinimumAmount(availableLeaves, targetAmount);
          if (minSelected !== null) {
            const reservedAmount = minSelected.reduce((sum, l) => sum + l.value, 0);
            const pendingChange = reservedAmount > targetAmount && targetAmount > 0
              ? reservedAmount - targetAmount
              : 0;

            const reservationId = this._generateId();
            await this._createReservation(client, reservationId, minSelected, purpose, pendingChange);

            return {
              type: "success",
              reservation: {
                id: reservationId,
                leaves: minSelected,
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
          "SELECT id FROM tree_reservations WHERE id = $1",
          [reservationId]
        );

        if (res.rows.length === 0) {
          throw new TreeStoreError(`Reservation ${reservationId} not found`);
        }

        // Get old reserved leaf IDs and mark as spent
        const oldLeavesResult = await client.query(
          "SELECT id FROM tree_leaves WHERE reservation_id = $1",
          [reservationId]
        );
        const oldLeafIds = oldLeavesResult.rows.map((r) => r.id);

        await this._batchInsertSpentLeaves(client, oldLeafIds);
        await client.query(
          "DELETE FROM tree_leaves WHERE reservation_id = $1",
          [reservationId]
        );

        // Upsert change leaves to available pool
        await this._batchUpsertLeaves(client, changeLeaves, false, null);

        // Upsert reserved leaves
        await this._batchUpsertLeaves(client, reservedLeaves, false, null);

        // Set reservation_id on reserved leaves
        const reservedLeafIds = reservedLeaves.map((l) => l.id);
        await this._batchSetReservationId(client, reservationId, reservedLeafIds);

        // Clear pending change amount
        await client.query(
          "UPDATE tree_reservations SET pending_change_amount = 0 WHERE id = $1",
          [reservationId]
        );

        return {
          id: reservationId,
          leaves: reservedLeaves,
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

  async getReservation(id) {
    try {
      const client = await this.pool.connect();
      try {
        // Check if reservation exists and hasn't expired
        const res = await client.query(
          `SELECT id FROM tree_reservations
           WHERE id = $1
             AND created_at >= NOW() - make_interval(secs => $2)`,
          [id, RESERVATION_TIMEOUT_SECS]
        );
        if (res.rows.length === 0) {
          throw new TreeStoreError(`Reservation ${id} not found or expired`);
        }

        // Get reserved leaves
        const leavesResult = await client.query(
          "SELECT data FROM tree_leaves WHERE reservation_id = $1",
          [id]
        );
        const leaves = leavesResult.rows.map((r) =>
          typeof r.data === "string" ? JSON.parse(r.data) : r.data
        );

        return { id, leaves };
      } finally {
        client.release();
      }
    } catch (error) {
      if (error instanceof TreeStoreError) throw error;
      throw new TreeStoreError(
        `Failed to get reservation '${id}': ${error.message}`,
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
      "SELECT COALESCE(SUM(pending_change_amount), 0)::BIGINT AS pending FROM tree_reservations"
    );
    return Number(result.rows[0].pending);
  }

  /**
   * Create a reservation with the given leaves.
   */
  async _createReservation(client, reservationId, leaves, purpose, pendingChange) {
    await client.query(
      "INSERT INTO tree_reservations (id, purpose, pending_change_amount) VALUES ($1, $2, $3)",
      [reservationId, purpose, pendingChange]
    );

    const leafIds = leaves.map((l) => l.id);
    await this._batchSetReservationId(client, reservationId, leafIds);
  }

  /**
   * Batch upsert leaves into tree_leaves table.
   */
  async _batchUpsertLeaves(client, leaves, isMissingFromOperators, skipIds) {
    if (!leaves || leaves.length === 0) return;

    const filtered = skipIds
      ? leaves.filter((l) => !skipIds.has(l.id))
      : leaves;

    if (filtered.length === 0) return;

    const ids = filtered.map((l) => l.id);
    const statuses = filtered.map((l) => l.status);
    const missingFlags = filtered.map(() => isMissingFromOperators);
    const dataValues = filtered.map((l) => JSON.stringify(l));

    await client.query(
      `INSERT INTO tree_leaves (id, status, is_missing_from_operators, data, added_at)
       SELECT id, status, missing, data::jsonb, NOW()
       FROM UNNEST($1::text[], $2::text[], $3::bool[], $4::text[])
           AS t(id, status, missing, data)
       ON CONFLICT (id) DO UPDATE SET
         status = EXCLUDED.status,
         is_missing_from_operators = EXCLUDED.is_missing_from_operators,
         data = EXCLUDED.data,
         added_at = NOW()`,
      [ids, statuses, missingFlags, dataValues]
    );
  }

  /**
   * Batch set reservation_id on leaves.
   */
  async _batchSetReservationId(client, reservationId, leafIds) {
    if (leafIds.length === 0) return;

    await client.query(
      "UPDATE tree_leaves SET reservation_id = $1 WHERE id = ANY($2)",
      [reservationId, leafIds]
    );
  }

  /**
   * Batch insert spent leaf markers.
   */
  async _batchInsertSpentLeaves(client, leafIds) {
    if (leafIds.length === 0) return;

    await client.query(
      "INSERT INTO tree_spent_leaves (leaf_id) SELECT * FROM UNNEST($1::text[]) ON CONFLICT DO NOTHING",
      [leafIds]
    );
  }

  /**
   * Batch remove spent leaf markers.
   */
  async _batchRemoveSpentLeaves(client, leafIds) {
    if (leafIds.length === 0) return;

    await client.query(
      "DELETE FROM tree_spent_leaves WHERE leaf_id = ANY($1)",
      [leafIds]
    );
  }

  /**
   * Clean up stale reservations.
   */
  async _cleanupStaleReservations(client) {
    await client.query(
      `DELETE FROM tree_reservations
       WHERE created_at < NOW() - make_interval(secs => $1)`,
      [RESERVATION_TIMEOUT_SECS]
    );
  }

  /**
   * Clean up old spent markers.
   */
  async _cleanupSpentMarkers(client, refreshTimestamp) {
    const thresholdMs = SPENT_MARKER_CLEANUP_THRESHOLD_MS;
    const cleanupCutoff = new Date(refreshTimestamp.getTime() - thresholdMs);

    await client.query(
      "DELETE FROM tree_spent_leaves WHERE spent_at < $1",
      [cleanupCutoff]
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
 * @param {object} [logger] - Optional logger
 * @returns {Promise<PostgresTreeStore>}
 */
async function createPostgresTreeStore(config, logger = null) {
  const pool = new pg.Pool({
    connectionString: config.connectionString,
    max: config.maxPoolSize,
    connectionTimeoutMillis: config.createTimeoutSecs * 1000,
    idleTimeoutMillis: config.recycleTimeoutSecs * 1000,
  });

  const store = new PostgresTreeStore(pool, logger);
  await store.initialize();
  return store;
}

module.exports = { PostgresTreeStore, createPostgresTreeStore, TreeStoreError };
