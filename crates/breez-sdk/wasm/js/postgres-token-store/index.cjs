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
 * Domain prefix mixed into the per-tenant advisory-lock key. Distinct prefixes
 * guarantee that locks from different stores (tree, token, …) never collide.
 */
const TOKEN_STORE_LOCK_PREFIX = "breez-spark-sdk:token:";

/**
 * Spent markers are kept for this duration to support multiple SDK instances.
 * During setTokensOutputs, spent markers older than refresh_timestamp are ignored.
 */
const SPENT_MARKER_CLEANUP_THRESHOLD_MS = 5 * 60 * 1000; // 5 minutes

/**
 * Reservations whose created_at is older than this are considered stale and are
 * dropped at the start of setTokensOutputs. Matches the tree store's timeout.
 */
const RESERVATION_TIMEOUT_SECS = 300; // 5 minutes

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

/**
 * Groups outputs by the token each one names, keeping first-seen token order.
 * @param {Array<Object>} outputs
 * @returns {Array<[string, Array<Object>]>}
 */
function _groupOutputsByToken(outputs) {
  const grouped = new Map();
  for (const output of outputs ?? []) {
    const identifier = output.output.tokenIdentifier;
    if (!grouped.has(identifier)) {
      grouped.set(identifier, []);
    }
    grouped.get(identifier).push(output);
  }
  return Array.from(grouped.entries());
}

/**
 * The metadata entry describing `identifier`, or null when absent.
 */
function _metadataFor(metadata, identifier) {
  return (metadata ?? []).find((m) => m.identifier === identifier) ?? null;
}

/** The distinct token identifiers the outputs belong to, in first-seen order. */
function _tokenIdentifiersOf(outputs) {
  return Array.from(new Set(outputs.map((o) => o.output.tokenIdentifier)));
}

/**
 * Rejects reservation targets that can never be satisfied.
 * @param {Array<[string, {type: string, value: string|number}]>} targets
 */
function _validateTargets(targets) {
  if (!targets || targets.length === 0) {
    throw new TokenStoreError("No reservation targets provided");
  }
  for (const [, target] of targets) {
    if (
      target.type === "minTotalValue" &&
      (!target.value || target.value === "0")
    ) {
      throw new TokenStoreError("Amount to reserve must be greater than zero");
    }
    if (target.type === "maxOutputCount" && !target.value) {
      throw new TokenStoreError("Count to reserve must be greater than zero");
    }
  }
}

class PostgresTokenStore {
  /**
   * @param {import('pg').Pool} pool
   * @param {Buffer|Uint8Array} identity - 33-byte secp256k1 compressed pubkey
   *   identifying the tenant. All reads and writes are scoped by this.
   * @param {object} [logger]
   */
  constructor(pool, identity, logger = null, runMigration = true) {
    if (!identity || identity.length !== 33) {
      throw new TokenStoreError(
        "tenant identity (33-byte secp256k1 pubkey) is required"
      );
    }
    this.pool = pool;
    this.identity = Buffer.from(identity);
    this.lockKey = _identityLockKey(TOKEN_STORE_LOCK_PREFIX, identity);
    this.logger = logger;
    this.runMigration = runMigration;
  }

  /**
   * Initialize the database (run migrations)
   */
  async initialize() {
    try {
      if (this.runMigration) {
        const migrationManager = new TokenStoreMigrationManager(this.logger);
        await migrationManager.migrate(this.pool, this.identity);
      }
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
   * Run a function inside a transaction with the advisory lock. Reserved for
   * operations whose correctness depends on serializing the available-output
   * set (`reserveTokenOutputs`, `setTokensOutputs`).
   * @param {function(import('pg').PoolClient): Promise<T>} fn
   * @returns {Promise<T>}
   * @template T
   */
  async _withWriteTransaction(fn) {
    const client = await this.pool.connect();
    try {
      await client.query("BEGIN");
      // Per-tenant advisory lock: 64-bit key derived from a token-store domain
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
   * Run a function inside a transaction without the advisory lock. Used by
   * operations scoped to a single reservation_id (`cancelReservation`)
   * where MVCC + row-level locks suffice and the global lock would only add
   * contention.
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

  // ===== TokenOutputStore Methods =====

  /**
   * Set the full set of token outputs, reconciling reservations.
   * @param {{metadata: Array<Object>, outputs: Array<Object>}} tokenOutputs - A flat
   *   output list that may span several tokens, plus one metadata entry per token
   *   the outputs name.
   * @param {number} refreshStartedAtMs - Milliseconds since epoch when the refresh started
   */
  async setTokensOutputs(tokenOutputs, refreshStartedAtMs) {
    try {
      const refreshTimestamp = new Date(refreshStartedAtMs);

      await this._withWriteTransaction(async (client) => {
        // Drop expired reservations BEFORE evaluating has_active_swap, otherwise a stale
        // Swap reservation (from a crashed client or a swap whose finalize/cancel never
        // ran) keeps has_active_swap true forever, which makes setTokensOutputs
        // early-return and never reach any subsequent reconciliation. The reservation
        // pins itself in place and the local token-output set freezes.
        await this._cleanupStaleReservations(client);

        // Skip if swap is active or completed during this refresh
        const swapCheckResult = await client.query(
          `SELECT
            EXISTS(
              SELECT 1 FROM brz_token_reservations
              WHERE user_id = $1 AND purpose = 'Swap'
            ) AS has_active_swap,
            COALESCE(
              (SELECT last_completed_at >= $2
               FROM brz_token_swap_status WHERE user_id = $1),
              FALSE
            ) AS swap_completed`,
          [this.identity, refreshTimestamp]
        );
        const { has_active_swap, swap_completed } = swapCheckResult.rows[0];
        if (has_active_swap || swap_completed) {
          return;
        }

        // Clean up old spent markers
        const cleanupCutoff = new Date(refreshTimestamp.getTime() - SPENT_MARKER_CLEANUP_THRESHOLD_MS);
        await client.query(
          "DELETE FROM brz_token_spent_outputs WHERE user_id = $1 AND spent_at < $2",
          [this.identity, cleanupCutoff]
        );

        // Get recent spent outpoints (spent_at >= refresh_timestamp)
        const spentResult = await client.query(
          "SELECT prev_tx_hash, prev_tx_vout FROM brz_token_spent_outputs WHERE user_id = $1 AND spent_at >= $2",
          [this.identity, refreshTimestamp]
        );
        const spentOutpoints = new Set(
          spentResult.rows.map((r) => `${r.prev_tx_hash}:${r.prev_tx_vout}`)
        );

        // Delete non-reserved outputs added BEFORE the refresh started
        await client.query(
          "DELETE FROM brz_token_outputs WHERE user_id = $1 AND reservation_id IS NULL AND added_at < $2",
          [this.identity, refreshTimestamp]
        );

        // Build a set of all incoming outpoints for reconciliation
        const incomingOutpoints = new Set(
          tokenOutputs.outputs.map((o) => `${o.prevTxHash}:${o.prevTxVout}`)
        );

        // Reconcile reservations: find reserved outputs that no longer exist
        const reservedRows = await client.query(
          `SELECT r.id, o.prev_tx_hash, o.prev_tx_vout
           FROM brz_token_reservations r
           JOIN brz_token_outputs o
             ON o.reservation_id = r.id AND o.user_id = r.user_id
           WHERE r.user_id = $1`,
          [this.identity]
        );

        // Group reserved outpoints by reservation ID
        const reservationOutputs = new Map();
        for (const row of reservedRows.rows) {
          if (!reservationOutputs.has(row.id)) {
            reservationOutputs.set(row.id, []);
          }
          reservationOutputs.get(row.id).push([row.prev_tx_hash, row.prev_tx_vout]);
        }

        // Find reservations that have no valid outputs after reconciliation
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

        // Delete outputs whose reservations are being removed entirely
        if (reservationsToDelete.length > 0) {
          await client.query(
            "DELETE FROM brz_token_outputs WHERE user_id = $1 AND reservation_id = ANY($2)",
            [this.identity, reservationsToDelete]
          );
          await client.query(
            "DELETE FROM brz_token_reservations WHERE user_id = $1 AND id = ANY($2)",
            [this.identity, reservationsToDelete]
          );
        }

        // Delete individual reserved outputs that no longer exist
        if (outpointsToRemoveFromReservation.length > 0) {
          const txHashes = outpointsToRemoveFromReservation.map(([h]) => h);
          const vouts = outpointsToRemoveFromReservation.map(([, v]) => v);
          await client.query(
            `DELETE FROM brz_token_outputs
             WHERE user_id = $1
               AND (prev_tx_hash, prev_tx_vout) IN (
                 SELECT * FROM UNNEST($2::text[], $3::int[])
               )`,
            [this.identity, txHashes, vouts]
          );

          // Check if any reservations are now empty
          const emptyReservations = await client.query(
            `SELECT r.id FROM brz_token_reservations r
             LEFT JOIN brz_token_outputs o
               ON o.reservation_id = r.id AND o.user_id = r.user_id
             WHERE r.user_id = $1 AND o.prev_tx_hash IS NULL`,
            [this.identity]
          );
          const emptyIds = emptyReservations.rows.map((r) => r.id);
          if (emptyIds.length > 0) {
            await client.query(
              "DELETE FROM brz_token_reservations WHERE user_id = $1 AND id = ANY($2)",
              [this.identity, emptyIds]
            );
          }
        }

        // Collect outpoints of currently reserved outputs (that survived reconciliation)
        const reservedOutpointsResult = await client.query(
          "SELECT prev_tx_hash, prev_tx_vout FROM brz_token_outputs WHERE user_id = $1 AND reservation_id IS NOT NULL",
          [this.identity]
        );
        const reservedOutpoints = new Set(
          reservedOutpointsResult.rows.map(
            (r) => `${r.prev_tx_hash}:${r.prev_tx_vout}`
          )
        );

        // Delete orphan metadata (per-tenant)
        await client.query(
          `DELETE FROM brz_token_metadata
           WHERE user_id = $1
             AND identifier NOT IN (
               SELECT DISTINCT token_identifier
               FROM brz_token_outputs WHERE user_id = $1
             )`,
          [this.identity]
        );

        // Insert new metadata and outputs, excluding spent and reserved
        for (const [tokenIdentifier, outputs] of _groupOutputsByToken(
          tokenOutputs.outputs
        )) {
          const metadata = _metadataFor(tokenOutputs.metadata, tokenIdentifier);
          if (!metadata) {
            this._log(
              "warn",
              `Skipping outputs of token ${tokenIdentifier}: no metadata provided`
            );
            continue;
          }
          await this._upsertMetadata(client, metadata);

          for (const output of outputs) {
            const outpoint = `${output.prevTxHash}:${output.prevTxVout}`;
            if (reservedOutpoints.has(outpoint) || spentOutpoints.has(outpoint)) {
              continue;
            }
            await this._insertSingleOutput(client, tokenIdentifier, output);
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
  /**
   * Returns the spendable per-token balances aggregated server-side.
   * Each entry includes full token metadata + the available + swap-reserved sum.
   * Matches the in-memory default impl which returns all tokens that have
   * at least one output (including zero spendable balance).
   * @returns {Promise<Array<{metadata: object, balance: string}>>}
   */
  async getTokenBalances() {
    try {
      const result = await this.pool.query(
        `
        SELECT m.identifier, m.issuer_public_key, m.name, m.ticker, m.decimals,
               m.max_supply, m.is_freezable, m.creation_entity_public_key,
               COALESCE(SUM(
                 CASE
                   WHEN o.reservation_id IS NULL THEN o.token_amount::numeric
                   WHEN r.purpose = 'Swap' THEN o.token_amount::numeric
                   ELSE 0
                 END
               ), 0)::text AS balance
        FROM brz_token_metadata m
        JOIN brz_token_outputs o
          ON o.token_identifier = m.identifier AND o.user_id = m.user_id
        LEFT JOIN brz_token_reservations r
          ON o.reservation_id = r.id AND o.user_id = r.user_id
        WHERE m.user_id = $1
        GROUP BY m.identifier, m.issuer_public_key, m.name, m.ticker,
                 m.decimals, m.max_supply, m.is_freezable, m.creation_entity_public_key
      `,
        [this.identity]
      );
      return result.rows.map((row) => ({
        metadata: {
          identifier: row.identifier,
          issuerPublicKey: row.issuer_public_key,
          name: row.name,
          ticker: row.ticker,
          decimals: row.decimals,
          maxSupply: row.max_supply,
          isFreezable: row.is_freezable,
          creationEntityPublicKey: row.creation_entity_public_key,
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
      const result = await this.pool.query(
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
         WHERE m.user_id = $1
         ORDER BY m.identifier, o.token_amount::NUMERIC ASC`,
        [this.identity]
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
         WHERE m.user_id = $2 AND ${whereClause}
         ORDER BY o.token_amount::NUMERIC ASC`,
        [param, this.identity]
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
   * @param {{metadata: Array<Object>, outputs: Array<Object>}} outputsToAdd - A flat
   *   output list that may span several tokens, plus one metadata entry per token
   *   the outputs name.
   * @returns {Promise<void>}
   */
  async updateTokenOutputs(outputsToRemove, outputsToAdd) {
    try {
      // Serialize against the other token-store mutators (refresh, reservation,
      // finalization), which take the same per-user advisory lock.
      await this._withWriteTransaction(async (client) => {
        // 1. Remove spent outputs and mark as spent.
        if (outputsToRemove && outputsToRemove.length > 0) {
          for (const [txHash, vout] of outputsToRemove) {
            const result = await client.query(
              "DELETE FROM brz_token_outputs WHERE user_id = $1 AND prev_tx_hash = $2 AND prev_tx_vout = $3",
              [this.identity, txHash, vout]
            );
            if (result.rowCount > 0) {
              await client.query(
                "INSERT INTO brz_token_spent_outputs (user_id, prev_tx_hash, prev_tx_vout, spent_at) VALUES ($1, $2, $3, NOW()) ON CONFLICT DO NOTHING",
                [this.identity, txHash, vout]
              );
            }
          }
        }

        // 2. Insert new outputs, grouped by the token each one names.
        for (const [tokenIdentifier, outputs] of _groupOutputsByToken(
          outputsToAdd?.outputs
        )) {
          const metadata = _metadataFor(outputsToAdd.metadata, tokenIdentifier);
          if (!metadata) {
            this._log(
              "warn",
              `Skipping outputs of token ${tokenIdentifier}: no metadata provided`
            );
            continue;
          }
          await this._upsertMetadata(client, metadata);

          // Clear spent status for outputs being (re-)added.
          const txHashes = outputs.map((o) => o.prevTxHash);
          const vouts = outputs.map((o) => o.prevTxVout);
          await client.query(
            `DELETE FROM brz_token_spent_outputs
             WHERE user_id = $1
               AND (prev_tx_hash, prev_tx_vout) IN (
                 SELECT * FROM UNNEST($2::text[], $3::int[])
               )`,
            [this.identity, txHashes, vouts]
          );

          for (const output of outputs) {
            await this._insertSingleOutput(client, tokenIdentifier, output);
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

  /**
   * Reserve token outputs for a payment or swap.
   *
   * Selection and reservation of every target share one transaction and the
   * write lock it holds, so a multi-token reservation is all-or-nothing: no
   * concurrent writer can take an output between two targets being selected.
   * @param {Array<[string, {type: string, value: string|number}]>} targets - One
   *   [tokenIdentifier, target] pair per token to reserve for, target being
   *   MinTotalValue or MaxOutputCount
   * @param {string} purpose - "Payment" or "Swap"
   * @param {Array|null} preferredOutputs
   * @param {string|null} selectionStrategy - "SmallestFirst" or "LargestFirst"
   * @returns {Promise<{id: string, tokenOutputs: {metadata: Array<Object>, outputs: Array}}>}
   */
  async reserveTokenOutputs(
    targets,
    purpose,
    preferredOutputs,
    selectionStrategy
  ) {
    try {
      _validateTargets(targets);

      return await this._withWriteTransaction(async (client) => {
        const selected = await this._selectForTargets(
          client,
          targets,
          preferredOutputs,
          selectionStrategy
        );

        const reservationId = this._generateId();
        await this._insertReservation(client, reservationId, purpose);
        await this._assignReservation(
          client,
          reservationId,
          selected.outputs
        );

        return { id: reservationId, tokenOutputs: selected };
      });
    } catch (error) {
      if (error instanceof TokenStoreError) throw error;
      throw new TokenStoreError(
        `Failed to reserve token outputs: ${error.message}`,
        error
      );
    }
  }

  _selectOutputs(tokenIdentifier, outputs, target, selectionStrategy) {
    if (target.type === "minTotalValue") {
      const amount = BigInt(target.value);
      const totalAvailable = outputs.reduce(
        (sum, o) => sum + BigInt(o.output.tokenAmount),
        0n
      );
      if (totalAvailable < amount) {
        throw new TokenStoreError(
          `InsufficientFunds: ${tokenIdentifier}`
        );
      }

      const exactMatch = outputs.find(
        (o) => BigInt(o.output.tokenAmount) === amount
      );
      if (exactMatch) {
        return [exactMatch];
      }

      if (selectionStrategy === "LargestFirst") {
        outputs.sort(
          (a, b) =>
            Number(BigInt(b.output.tokenAmount) - BigInt(a.output.tokenAmount))
        );
      } else {
        outputs.sort(
          (a, b) =>
            Number(BigInt(a.output.tokenAmount) - BigInt(b.output.tokenAmount))
        );
      }

      const selected = [];
      let remaining = amount;
      for (const output of outputs) {
        if (remaining <= 0n) break;
        selected.push(output);
        remaining -= BigInt(output.output.tokenAmount);
      }
      if (remaining > 0n) {
        throw new TokenStoreError(
          `InsufficientFunds: ${tokenIdentifier}`
        );
      }
      return selected;
    }

    if (target.type === "maxOutputCount") {
      const count = target.value;
      if (selectionStrategy === "LargestFirst") {
        outputs.sort(
          (a, b) =>
            Number(BigInt(b.output.tokenAmount) - BigInt(a.output.tokenAmount))
        );
      } else {
        outputs.sort(
          (a, b) =>
            Number(BigInt(a.output.tokenAmount) - BigInt(b.output.tokenAmount))
        );
      }
      return outputs.slice(0, count);
    }

    throw new TokenStoreError(`Unknown target type: ${target.type}`);
  }

  /**
   * Select outputs covering every target, without reserving them.
   * @param {Array<[string, {type: string, value: string|number}]>} targets - One
   *   [tokenIdentifier, target] pair per token to select for
   * @param {Array|null} preferredOutputs
   * @param {string|null} selectionStrategy - "SmallestFirst" or "LargestFirst"
   * @returns {Promise<{metadata: Array<Object>, outputs: Array}>}
   */
  async selectTokenOutputs(targets, preferredOutputs, selectionStrategy) {
    try {
      _validateTargets(targets);

      return await this._selectForTargets(
        this.pool,
        targets,
        preferredOutputs,
        selectionStrategy
      );
    } catch (error) {
      if (error instanceof TokenStoreError) throw error;
      throw new TokenStoreError(
        `Failed to select token outputs: ${error.message}`,
        error
      );
    }
  }

  /**
   * Reserve the outputs at the given outpoints, which may belong to several tokens.
   * @param {Array<{prevTxHash: string, prevTxVout: number}>} outpoints
   * @param {string} purpose - "Payment" or "Swap"
   * @returns {Promise<{id: string, tokenOutputs: {metadata: Array<Object>, outputs: Array}}>}
   */
  async reserveTokenOutputsByOutpoints(outpoints, purpose) {
    try {
      if (!outpoints || outpoints.length === 0) {
        throw new TokenStoreError("No outpoints provided");
      }
      return await this._withWriteTransaction(async (client) => {
        const txHashes = outpoints.map((o) => o.prevTxHash);
        const vouts = outpoints.map((o) => o.prevTxVout);

        // The outpoints are not scoped to a token: they may belong to several.
        const outputRows = await client.query(
          `SELECT o.owner_public_key, o.revocation_commitment,
                  o.withdraw_bond_sats, o.withdraw_relative_block_locktime,
                  o.token_public_key, o.token_amount, o.token_identifier,
                  o.prev_tx_hash, o.prev_tx_vout
           FROM brz_token_outputs o
           WHERE o.user_id = $1
             AND o.reservation_id IS NULL
             AND (o.prev_tx_hash, o.prev_tx_vout) IN (
               SELECT * FROM UNNEST($2::text[], $3::int[])
             )`,
          [this.identity, txHashes, vouts]
        );

        const selectedOutputs = outputRows.rows.map((row) =>
          this._outputFromRow(row)
        );

        const distinct = new Set(
          outpoints.map((o) => `${o.prevTxHash}:${o.prevTxVout}`)
        );
        if (selectedOutputs.length !== distinct.size) {
          throw new TokenStoreError("InsufficientFunds");
        }

        const metadata = await this._fetchMetadata(
          client,
          _tokenIdentifiersOf(selectedOutputs)
        );

        const reservationId = this._generateId();
        await this._insertReservation(client, reservationId, purpose);
        await this._assignReservation(client, reservationId, selectedOutputs);

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
        `Failed to reserve token outputs by outpoints: ${error.message}`,
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
      await this._withTransaction(async (client) => {
        // Clear reservation_id from outputs first — the composite FK uses NO
        // ACTION (column-list SET NULL is PG15+ and a whole-row SET NULL would
        // null user_id, which is NOT NULL).
        await client.query(
          "UPDATE brz_token_outputs SET reservation_id = NULL WHERE user_id = $1 AND reservation_id = $2",
          [this.identity, id]
        );

        // Delete the reservation
        await client.query(
          "DELETE FROM brz_token_reservations WHERE user_id = $1 AND id = $2",
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

  /**
   * Finalize a reservation, deleting reserved outputs and cleaning up.
   * @param {string} id - Reservation ID
   */
  async finalizeReservation(id) {
    try {
      // _withWriteTransaction acquires the advisory lock so this serializes
      // against `setTokensOutputs`. Without it, a concurrent setTokensOutputs
      // could read brz_token_spent_outputs before our marker commits and re-insert
      // the just-spent output as Available.
      await this._withWriteTransaction(async (client) => {
        // Get reservation purpose
        const reservationResult = await client.query(
          "SELECT purpose FROM brz_token_reservations WHERE user_id = $1 AND id = $2",
          [this.identity, id]
        );
        if (reservationResult.rows.length === 0) {
          return; // Non-existing reservation
        }
        const isSwap = reservationResult.rows[0].purpose === "Swap";

        // Get reserved outpoints and mark them as spent
        const reservedOutputsResult = await client.query(
          "SELECT prev_tx_hash, prev_tx_vout FROM brz_token_outputs WHERE user_id = $1 AND reservation_id = $2",
          [this.identity, id]
        );

        if (reservedOutputsResult.rows.length > 0) {
          const txHashes = reservedOutputsResult.rows.map((r) => r.prev_tx_hash);
          const vouts = reservedOutputsResult.rows.map((r) => r.prev_tx_vout);
          await client.query(
            `INSERT INTO brz_token_spent_outputs (user_id, prev_tx_hash, prev_tx_vout)
             SELECT $3, h, v FROM UNNEST($1::text[], $2::int[]) AS t(h, v)
             ON CONFLICT DO NOTHING`,
            [txHashes, vouts, this.identity]
          );
        }

        // Delete reserved outputs
        await client.query(
          "DELETE FROM brz_token_outputs WHERE user_id = $1 AND reservation_id = $2",
          [this.identity, id]
        );

        // Delete the reservation
        await client.query(
          "DELETE FROM brz_token_reservations WHERE user_id = $1 AND id = $2",
          [this.identity, id]
        );

        // If this was a swap reservation, update last_completed_at. UPSERT so a
        // tenant that joined after migration 2 (and thus has no row) gets one.
        if (isSwap) {
          await client.query(
            `INSERT INTO brz_token_swap_status (user_id, last_completed_at)
             VALUES ($1, NOW())
             ON CONFLICT (user_id) DO UPDATE
               SET last_completed_at = EXCLUDED.last_completed_at`,
            [this.identity]
          );
        }

        // Clean up orphaned metadata (per-tenant)
        await client.query(
          `DELETE FROM brz_token_metadata
           WHERE user_id = $1
             AND identifier NOT IN (
               SELECT DISTINCT token_identifier
               FROM brz_token_outputs WHERE user_id = $1
             )`,
          [this.identity]
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

  _log(level, message) {
    if (this.logger && typeof this.logger.log === "function") {
      this.logger.log({ line: message, level });
    }
  }

  /**
   * Select available outputs covering every target, without reserving them.
   *
   * The result carries metadata for every requested token. A target whose token
   * is unknown to this tenant is an error, and a target that cannot be covered
   * by the available outputs yields `InsufficientFunds`.
   * @param {import('pg').PoolClient|import('pg').Pool} client
   * @returns {Promise<{metadata: Array<Object>, outputs: Array}>}
   */
  async _selectForTargets(client, targets, preferredOutputs, selectionStrategy) {
    const tokenIdentifiers = Array.from(new Set(targets.map(([id]) => id)));

    const metadata = await this._fetchMetadata(client, tokenIdentifiers);
    for (const identifier of tokenIdentifiers) {
      if (!metadata.some((m) => m.identifier === identifier)) {
        throw new TokenStoreError(
          `Token outputs not found for identifier: ${identifier}`
        );
      }
    }

    const outputRows = await client.query(
      `SELECT o.owner_public_key, o.revocation_commitment,
              o.withdraw_bond_sats, o.withdraw_relative_block_locktime,
              o.token_public_key, o.token_amount, o.token_identifier,
              o.prev_tx_hash, o.prev_tx_vout
       FROM brz_token_outputs o
       WHERE o.user_id = $1
         AND o.token_identifier = ANY($2)
         AND o.reservation_id IS NULL`,
      [this.identity, tokenIdentifiers]
    );

    let available = outputRows.rows.map((row) => this._outputFromRow(row));

    if (preferredOutputs) {
      const preferredOutpoints = new Set(
        preferredOutputs.map((p) => `${p.prevTxHash}:${p.prevTxVout}`)
      );
      available = available.filter((o) =>
        preferredOutpoints.has(`${o.prevTxHash}:${o.prevTxVout}`)
      );
    }

    const availablePerToken = new Map();
    for (const output of available) {
      const identifier = output.output.tokenIdentifier;
      if (!availablePerToken.has(identifier)) {
        availablePerToken.set(identifier, []);
      }
      availablePerToken.get(identifier).push(output);
    }

    const outputs = [];
    for (const [tokenIdentifier, target] of targets) {
      const candidates = availablePerToken.get(tokenIdentifier) ?? [];
      const selected = this._selectOutputs(
        tokenIdentifier,
        candidates,
        target,
        selectionStrategy
      );

      // Outputs picked for one target are withheld from the next, so repeated
      // entries for the same token do not select the same output twice.
      const taken = new Set(
        selected.map((o) => `${o.prevTxHash}:${o.prevTxVout}`)
      );
      availablePerToken.set(
        tokenIdentifier,
        candidates.filter((o) => !taken.has(`${o.prevTxHash}:${o.prevTxVout}`))
      );

      outputs.push(...selected);
    }

    return { metadata, outputs };
  }

  /**
   * Load this tenant's metadata for the given token identifiers. Unknown
   * identifiers are simply absent from the result.
   */
  async _fetchMetadata(client, tokenIdentifiers) {
    if (tokenIdentifiers.length === 0) {
      return [];
    }
    const result = await client.query(
      "SELECT * FROM brz_token_metadata WHERE user_id = $1 AND identifier = ANY($2)",
      [this.identity, tokenIdentifiers]
    );
    return result.rows.map((row) => this._metadataFromRow(row));
  }

  async _insertReservation(client, reservationId, purpose) {
    await client.query(
      "INSERT INTO brz_token_reservations (user_id, id, purpose) VALUES ($1, $2, $3)",
      [this.identity, reservationId, purpose]
    );
  }

  /**
   * Point the given outputs at the reservation, addressing them by outpoint.
   */
  async _assignReservation(client, reservationId, outputs) {
    if (outputs.length === 0) {
      return;
    }
    const txHashes = outputs.map((o) => o.prevTxHash);
    const vouts = outputs.map((o) => o.prevTxVout);
    await client.query(
      `UPDATE brz_token_outputs SET reservation_id = $1
       WHERE user_id = $4
         AND (prev_tx_hash, prev_tx_vout) IN (
           SELECT * FROM UNNEST($2::text[], $3::int[])
         )`,
      [reservationId, txHashes, vouts, this.identity]
    );
  }

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
   * Delete reservations that have exceeded the timeout. Releases outputs by
   * clearing reservation_id explicitly, then deletes the parents — the
   * composite FK uses NO ACTION (column-list SET NULL is PG15+ and a
   * whole-row SET NULL would null user_id, NOT NULL).
   */
  async _cleanupStaleReservations(client) {
    await client.query(
      `UPDATE brz_token_outputs SET reservation_id = NULL
       WHERE user_id = $2
         AND reservation_id IN (
           SELECT id FROM brz_token_reservations
           WHERE user_id = $2
             AND created_at < NOW() - make_interval(secs => $1)
         )`,
      [RESERVATION_TIMEOUT_SECS, this.identity]
    );
    await client.query(
      `DELETE FROM brz_token_reservations
       WHERE user_id = $2
         AND created_at < NOW() - make_interval(secs => $1)`,
      [RESERVATION_TIMEOUT_SECS, this.identity]
    );
  }

  /**
   * Upsert token metadata.
   */
  async _upsertMetadata(client, metadata) {
    await client.query(
      `INSERT INTO brz_token_metadata
        (user_id, identifier, issuer_public_key, name, ticker, decimals, max_supply,
         is_freezable, creation_entity_public_key)
       VALUES ($9, $1, $2, $3, $4, $5, $6, $7, $8)
       ON CONFLICT (user_id, identifier) DO UPDATE SET
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
        this.identity,
      ]
    );
  }

  /**
   * Insert a single output.
   */
  async _insertSingleOutput(client, tokenIdentifier, output) {
    await client.query(
      `INSERT INTO brz_token_outputs
        (user_id, token_identifier, owner_public_key, revocation_commitment,
         withdraw_bond_sats, withdraw_relative_block_locktime,
         token_public_key, token_amount, prev_tx_hash, prev_tx_vout, added_at)
       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW())
       ON CONFLICT (user_id, prev_tx_hash, prev_tx_vout) DO NOTHING`,
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
 * @param {Buffer|Uint8Array} identity - 33-byte secp256k1 compressed pubkey scoping reads/writes
 * @param {object} [logger] - Optional logger
 * @returns {Promise<PostgresTokenStore>}
 */
async function createPostgresTokenStore(config, identity, logger = null) {
  const pool = new pg.Pool({
    connectionString: config.connectionString,
    max: config.maxPoolSize,
    connectionTimeoutMillis: config.createTimeoutSecs * 1000,
    idleTimeoutMillis: config.recycleTimeoutSecs * 1000,
  });
  return createPostgresTokenStoreWithPool(
    pool,
    identity,
    logger,
    config.runMigration !== false
  );
}

/**
 * Create a PostgresTokenStore instance from an existing pg.Pool.
 *
 * @param {pg.Pool} pool - An existing connection pool
 * @param {Buffer|Uint8Array} identity - 33-byte secp256k1 compressed pubkey scoping reads/writes
 * @param {object} [logger] - Optional logger
 * @returns {Promise<PostgresTokenStore>}
 */
async function createPostgresTokenStoreWithPool(
  pool,
  identity,
  logger = null,
  runMigration = true
) {
  const store = new PostgresTokenStore(
    pool,
    identity,
    logger,
    runMigration
  );
  await store.initialize();
  return store;
}

module.exports = { PostgresTokenStore, createPostgresTokenStore, createPostgresTokenStoreWithPool, TokenStoreError };
