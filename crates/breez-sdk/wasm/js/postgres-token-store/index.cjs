/**
 * CommonJS implementation for Node.js PostgreSQL Token Store
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

const { TokenStoreError } = require("./errors.cjs");
const { TokenStoreMigrationManager } = require("./migrations.cjs");

/**
 * Advisory lock key for serializing token store write operations.
 * Matches the Rust constant TOKEN_STORE_WRITE_LOCK_KEY = 0x746F_6B65_6E53_5452
 */
const TOKEN_STORE_WRITE_LOCK_KEY = "8390042714201347154"; // 0x746F6B656E535452 as decimal string

/**
 * Spent markers are kept for this duration to support multiple SDK instances.
 * During setTokensOutputs, spent markers older than refresh_timestamp are ignored.
 */
const SPENT_MARKER_CLEANUP_THRESHOLD_MS = 5 * 60 * 1000; // 5 minutes

class PostgresTokenStore {
  constructor(pool, logger = null) {
    this.pool = pool;
    this.logger = logger;
  }

  /**
   * Initialize the database (run migrations)
   */
  async initialize() {
    try {
      const migrationManager = new TokenStoreMigrationManager(this.logger);
      await migrationManager.migrate(this.pool);
      return this;
    } catch (error) {
      throw new TokenStoreError(
        `Failed to initialize PostgreSQL token store: ${error.message}`,
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
      await client.query(`SELECT pg_advisory_xact_lock(${TOKEN_STORE_WRITE_LOCK_KEY})`);
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

  // ===== TokenOutputStore Methods =====

  /**
   * Set the full set of token outputs, reconciling reservations.
   * @param {Array<{metadata: Object, outputs: Array}>} tokenOutputs
   * @param {number} refreshStartedAtMs - Milliseconds since epoch when the refresh started
   */
  async setTokensOutputs(tokenOutputs, refreshStartedAtMs) {
    try {
      const refreshTimestamp = new Date(refreshStartedAtMs);

      await this._withWriteTransaction(async (client) => {
        // Skip if swap is active or completed during this refresh
        const swapCheckResult = await client.query(
          `SELECT
            EXISTS(SELECT 1 FROM token_reservations WHERE purpose = 'Swap') AS has_active_swap,
            COALESCE((SELECT last_completed_at >= $1 FROM token_swap_status WHERE id = 1), FALSE) AS swap_completed`,
          [refreshTimestamp]
        );
        const { has_active_swap, swap_completed } = swapCheckResult.rows[0];
        if (has_active_swap || swap_completed) {
          return;
        }

        // Clean up old spent markers
        const cleanupCutoff = new Date(refreshTimestamp.getTime() - SPENT_MARKER_CLEANUP_THRESHOLD_MS);
        await client.query(
          "DELETE FROM token_spent_outputs WHERE spent_at < $1",
          [cleanupCutoff]
        );

        // Get recent spent output IDs (spent_at >= refresh_timestamp)
        const spentResult = await client.query(
          "SELECT output_id FROM token_spent_outputs WHERE spent_at >= $1",
          [refreshTimestamp]
        );
        const spentIds = new Set(spentResult.rows.map((r) => r.output_id));

        // Delete non-reserved outputs added BEFORE the refresh started
        await client.query(
          "DELETE FROM token_outputs WHERE reservation_id IS NULL AND added_at < $1",
          [refreshTimestamp]
        );

        // Build a set of all incoming output IDs for reconciliation
        const incomingOutputIds = new Set();
        for (const to of tokenOutputs) {
          for (const o of to.outputs) {
            incomingOutputIds.add(o.output.id);
          }
        }

        // Reconcile reservations: find reserved outputs that no longer exist
        const reservedRows = await client.query(
          `SELECT r.id, o.id AS output_id
           FROM token_reservations r
           JOIN token_outputs o ON o.reservation_id = r.id`
        );

        // Group reserved outputs by reservation ID
        const reservationOutputs = new Map();
        for (const row of reservedRows.rows) {
          if (!reservationOutputs.has(row.id)) {
            reservationOutputs.set(row.id, []);
          }
          reservationOutputs.get(row.id).push(row.output_id);
        }

        // Find reservations that have no valid outputs after reconciliation
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

        // Delete outputs whose reservations are being removed entirely
        if (reservationsToDelete.length > 0) {
          await client.query(
            "DELETE FROM token_outputs WHERE reservation_id = ANY($1)",
            [reservationsToDelete]
          );
          await client.query(
            "DELETE FROM token_reservations WHERE id = ANY($1)",
            [reservationsToDelete]
          );
        }

        // Delete individual reserved outputs that no longer exist
        if (outputsToRemoveFromReservation.length > 0) {
          await client.query(
            "DELETE FROM token_outputs WHERE id = ANY($1)",
            [outputsToRemoveFromReservation]
          );

          // Check if any reservations are now empty
          const emptyReservations = await client.query(
            `SELECT r.id FROM token_reservations r
             LEFT JOIN token_outputs o ON o.reservation_id = r.id
             WHERE o.id IS NULL`
          );
          const emptyIds = emptyReservations.rows.map((r) => r.id);
          if (emptyIds.length > 0) {
            await client.query(
              "DELETE FROM token_reservations WHERE id = ANY($1)",
              [emptyIds]
            );
          }
        }

        // Collect IDs of currently reserved outputs (that survived reconciliation)
        const reservedOutputIdsResult = await client.query(
          "SELECT id FROM token_outputs WHERE reservation_id IS NOT NULL"
        );
        const reservedOutputIds = new Set(
          reservedOutputIdsResult.rows.map((r) => r.id)
        );

        // Delete orphan metadata
        await client.query(
          `DELETE FROM token_metadata
           WHERE identifier NOT IN (
             SELECT DISTINCT token_identifier FROM token_outputs
           )`
        );

        // Insert new metadata and outputs, excluding spent and reserved
        for (const to of tokenOutputs) {
          await this._upsertMetadata(client, to.metadata);

          for (const output of to.outputs) {
            if (reservedOutputIds.has(output.output.id) || spentIds.has(output.output.id)) {
              continue;
            }
            await this._insertSingleOutput(
              client,
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
   * List all token outputs grouped by status.
   * @returns {Promise<Array<{metadata: Object, available: Array, reservedForPayment: Array, reservedForSwap: Array}>>}
   */
  async listTokensOutputs() {
    try {
      const result = await this.pool.query(
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
         ORDER BY m.identifier, o.token_amount ASC`
      );

      const map = new Map();

      for (const row of result.rows) {
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

  /**
   * Get token outputs for a specific token by filter.
   * @param {{type: string, identifier?: string, issuerPublicKey?: string}} filter
   * @returns {Promise<{metadata: Object, available: Array, reservedForPayment: Array, reservedForSwap: Array}>}
   */
  async getTokenOutputs(filter) {
    try {
      let whereClause;
      let param;

      if (filter.type === "identifier") {
        whereClause = "m.identifier = $1";
        param = filter.identifier;
      } else if (filter.type === "issuerPublicKey") {
        whereClause = "m.issuer_public_key = $1";
        param = filter.issuerPublicKey;
      } else {
        throw new TokenStoreError(`Unknown filter type: ${filter.type}`);
      }

      const result = await this.pool.query(
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
         WHERE ${whereClause}`,
        [param]
      );

      if (result.rows.length === 0) {
        throw new TokenStoreError("Token outputs not found");
      }

      const metadata = this._metadataFromRow(result.rows[0]);
      const entry = {
        metadata,
        available: [],
        reservedForPayment: [],
        reservedForSwap: [],
      };

      for (const row of result.rows) {
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

  /**
   * Insert token outputs (upsert metadata, insert outputs with ON CONFLICT DO NOTHING).
   * @param {{metadata: Object, outputs: Array}} tokenOutputs
   */
  async insertTokenOutputs(tokenOutputs) {
    try {
      const client = await this.pool.connect();
      try {
        await client.query("BEGIN");

        await this._upsertMetadata(client, tokenOutputs.metadata);

        // Remove inserted output IDs from spent markers (output returned to us)
        const outputIds = tokenOutputs.outputs.map((o) => o.output.id);
        if (outputIds.length > 0) {
          await client.query(
            "DELETE FROM token_spent_outputs WHERE output_id = ANY($1)",
            [outputIds]
          );
        }

        for (const output of tokenOutputs.outputs) {
          await this._insertSingleOutput(
            client,
            tokenOutputs.metadata.identifier,
            output
          );
        }

        await client.query("COMMIT");
      } catch (error) {
        await client.query("ROLLBACK").catch(() => {});
        throw error;
      } finally {
        client.release();
      }
    } catch (error) {
      if (error instanceof TokenStoreError) throw error;
      throw new TokenStoreError(
        `Failed to insert token outputs: ${error.message}`,
        error
      );
    }
  }

  /**
   * Reserve token outputs for a payment or swap.
   * @param {string} tokenIdentifier
   * @param {{type: string, value: number}} target - MinTotalValue or MaxOutputCount
   * @param {string} purpose - "Payment" or "Swap"
   * @param {Array|null} preferredOutputs
   * @param {string|null} selectionStrategy - "SmallestFirst" or "LargestFirst"
   * @returns {Promise<{id: string, tokenOutputs: {metadata: Object, outputs: Array}}>}
   */
  async reserveTokenOutputs(
    tokenIdentifier,
    target,
    purpose,
    preferredOutputs,
    selectionStrategy
  ) {
    try {
      return await this._withWriteTransaction(async (client) => {
        // Validate target
        if (target.type === "minTotalValue" && (!target.value || target.value === "0")) {
          throw new TokenStoreError(
            "Amount to reserve must be greater than zero"
          );
        }
        if (target.type === "maxOutputCount" && (!target.value || target.value === 0)) {
          throw new TokenStoreError(
            "Count to reserve must be greater than zero"
          );
        }

        // Get metadata
        const metadataResult = await client.query(
          "SELECT * FROM token_metadata WHERE identifier = $1",
          [tokenIdentifier]
        );

        if (metadataResult.rows.length === 0) {
          throw new TokenStoreError(
            `Token outputs not found for identifier: ${tokenIdentifier}`
          );
        }

        const metadata = this._metadataFromRow(metadataResult.rows[0]);

        // Get available (non-reserved) outputs
        const outputRows = await client.query(
          `SELECT o.id AS output_id, o.owner_public_key, o.revocation_commitment,
                  o.withdraw_bond_sats, o.withdraw_relative_block_locktime,
                  o.token_public_key, o.token_amount, o.token_identifier,
                  o.prev_tx_hash, o.prev_tx_vout
           FROM token_outputs o
           WHERE o.token_identifier = $1 AND o.reservation_id IS NULL`,
          [tokenIdentifier]
        );

        let outputs = outputRows.rows.map((row) => this._outputFromRow(row));

        // Filter by preferred if provided
        if (preferredOutputs && preferredOutputs.length > 0) {
          const preferredIds = new Set(
            preferredOutputs.map((p) => p.output.id)
          );
          outputs = outputs.filter((o) => preferredIds.has(o.output.id));
        }

        // Select outputs based on target
        let selectedOutputs;

        if (target.type === "minTotalValue") {
          const amount = BigInt(target.value);

          // Check sufficiency
          const totalAvailable = outputs.reduce(
            (sum, o) => sum + BigInt(o.output.tokenAmount),
            0n
          );
          if (totalAvailable < amount) {
            throw new TokenStoreError("InsufficientFunds");
          }

          // Try exact match first
          const exactMatch = outputs.find(
            (o) => BigInt(o.output.tokenAmount) === amount
          );
          if (exactMatch) {
            selectedOutputs = [exactMatch];
          } else {
            // Sort by selection strategy
            if (selectionStrategy === "LargestFirst") {
              outputs.sort(
                (a, b) =>
                  Number(BigInt(b.output.tokenAmount) - BigInt(a.output.tokenAmount))
              );
            } else {
              // Default: SmallestFirst
              outputs.sort(
                (a, b) =>
                  Number(BigInt(a.output.tokenAmount) - BigInt(b.output.tokenAmount))
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

          // Sort by selection strategy
          if (selectionStrategy === "LargestFirst") {
            outputs.sort(
              (a, b) =>
                Number(BigInt(b.output.tokenAmount) - BigInt(a.output.tokenAmount))
            );
          } else {
            // Default: SmallestFirst
            outputs.sort(
              (a, b) =>
                Number(BigInt(a.output.tokenAmount) - BigInt(b.output.tokenAmount))
            );
          }

          selectedOutputs = outputs.slice(0, count);
        } else {
          throw new TokenStoreError(`Unknown target type: ${target.type}`);
        }

        // Create reservation
        const reservationId = this._generateId();

        await client.query(
          "INSERT INTO token_reservations (id, purpose) VALUES ($1, $2)",
          [reservationId, purpose]
        );

        // Set reservation_id on selected outputs
        const selectedIds = selectedOutputs.map((o) => o.output.id);
        if (selectedIds.length > 0) {
          await client.query(
            "UPDATE token_outputs SET reservation_id = $1 WHERE id = ANY($2)",
            [reservationId, selectedIds]
          );
        }

        return {
          id: reservationId,
          tokenOutputs: {
            metadata,
            outputs: selectedOutputs,
          },
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

  /**
   * Cancel a reservation, releasing reserved outputs.
   * @param {string} id - Reservation ID
   */
  async cancelReservation(id) {
    try {
      await this._withWriteTransaction(async (client) => {
        // Clear reservation_id from outputs
        await client.query(
          "UPDATE token_outputs SET reservation_id = NULL WHERE reservation_id = $1",
          [id]
        );

        // Delete the reservation
        await client.query(
          "DELETE FROM token_reservations WHERE id = $1",
          [id]
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

  /**
   * Finalize a reservation, deleting reserved outputs and cleaning up.
   * @param {string} id - Reservation ID
   */
  async finalizeReservation(id) {
    try {
      await this._withWriteTransaction(async (client) => {
        // Get reservation purpose
        const reservationResult = await client.query(
          "SELECT purpose FROM token_reservations WHERE id = $1",
          [id]
        );
        if (reservationResult.rows.length === 0) {
          return; // Non-existing reservation
        }
        const isSwap = reservationResult.rows[0].purpose === "Swap";

        // Get reserved output IDs and mark them as spent
        const reservedOutputsResult = await client.query(
          "SELECT id FROM token_outputs WHERE reservation_id = $1",
          [id]
        );
        const reservedOutputIds = reservedOutputsResult.rows.map((r) => r.id);

        if (reservedOutputIds.length > 0) {
          await client.query(
            `INSERT INTO token_spent_outputs (output_id)
             SELECT * FROM UNNEST($1::text[])
             ON CONFLICT DO NOTHING`,
            [reservedOutputIds]
          );
        }

        // Delete reserved outputs
        await client.query(
          "DELETE FROM token_outputs WHERE reservation_id = $1",
          [id]
        );

        // Delete the reservation
        await client.query(
          "DELETE FROM token_reservations WHERE id = $1",
          [id]
        );

        // If this was a swap reservation, update last_completed_at
        if (isSwap) {
          await client.query(
            "UPDATE token_swap_status SET last_completed_at = NOW() WHERE id = 1"
          );
        }

        // Clean up orphaned metadata
        await client.query(
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

  /**
   * Get the current database server time as milliseconds since epoch.
   * @returns {Promise<number>}
   */
  async now() {
    try {
      const result = await this.pool.query("SELECT NOW()");
      return result.rows[0].now.getTime();
    } catch (error) {
      if (error instanceof TokenStoreError) throw error;
      throw new TokenStoreError(
        `Failed to get current time: ${error.message}`,
        error
      );
    }
  }

  // ===== Private Helpers =====

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
   * Upsert token metadata.
   */
  async _upsertMetadata(client, metadata) {
    await client.query(
      `INSERT INTO token_metadata
        (identifier, issuer_public_key, name, ticker, decimals, max_supply,
         is_freezable, creation_entity_public_key)
       VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
       ON CONFLICT (identifier) DO UPDATE SET
         issuer_public_key = EXCLUDED.issuer_public_key,
         name = EXCLUDED.name,
         ticker = EXCLUDED.ticker,
         decimals = EXCLUDED.decimals,
         max_supply = EXCLUDED.max_supply,
         is_freezable = EXCLUDED.is_freezable,
         creation_entity_public_key = EXCLUDED.creation_entity_public_key`,
      [
        metadata.identifier,
        metadata.issuerPublicKey,
        metadata.name,
        metadata.ticker,
        metadata.decimals,
        metadata.maxSupply,
        metadata.isFreezable,
        metadata.creationEntityPublicKey || null,
      ]
    );
  }

  /**
   * Insert a single output.
   */
  async _insertSingleOutput(client, tokenIdentifier, output) {
    await client.query(
      `INSERT INTO token_outputs
        (id, token_identifier, owner_public_key, revocation_commitment,
         withdraw_bond_sats, withdraw_relative_block_locktime,
         token_public_key, token_amount, prev_tx_hash, prev_tx_vout, added_at)
       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW())
       ON CONFLICT (id) DO NOTHING`,
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

  /**
   * Parse a TokenMetadata from a database row.
   */
  _metadataFromRow(row) {
    return {
      identifier: row.identifier,
      issuerPublicKey: row.issuer_public_key,
      name: row.name,
      ticker: row.ticker,
      decimals: row.decimals,
      maxSupply: row.max_supply,
      isFreezable: row.is_freezable,
      creationEntityPublicKey: row.creation_entity_public_key || null,
    };
  }

  /**
   * Parse a TokenOutputWithPrevOut from a database row.
   */
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

/**
 * Create a PostgresTokenStore instance from a config object.
 *
 * @param {object} config - PostgreSQL configuration
 * @param {string} config.connectionString - PostgreSQL connection string
 * @param {number} config.maxPoolSize - Maximum number of connections in the pool
 * @param {number} config.createTimeoutSecs - Timeout in seconds for establishing a new connection
 * @param {number} config.recycleTimeoutSecs - Timeout in seconds before recycling an idle connection
 * @param {object} [logger] - Optional logger
 * @returns {Promise<PostgresTokenStore>}
 */
async function createPostgresTokenStore(config, logger = null) {
  const pool = new pg.Pool({
    connectionString: config.connectionString,
    max: config.maxPoolSize,
    connectionTimeoutMillis: config.createTimeoutSecs * 1000,
    idleTimeoutMillis: config.recycleTimeoutSecs * 1000,
  });
  return createPostgresTokenStoreWithPool(pool, logger);
}

/**
 * Create a PostgresTokenStore instance from an existing pg.Pool.
 *
 * @param {pg.Pool} pool - An existing connection pool
 * @param {object} [logger] - Optional logger
 * @returns {Promise<PostgresTokenStore>}
 */
async function createPostgresTokenStoreWithPool(pool, logger = null) {
  const store = new PostgresTokenStore(pool, logger);
  await store.initialize();
  return store;
}

module.exports = { PostgresTokenStore, createPostgresTokenStore, createPostgresTokenStoreWithPool, TokenStoreError };
