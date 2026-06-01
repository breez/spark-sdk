/**
 * Token store migrations for MySQL 8.0+. Mirrors `postgres-token-store/migrations.cjs`.
 */

const { TokenStoreError } = require("./errors.cjs");

const TOKEN_MIGRATIONS_TABLE = "brz_token_schema_migrations";
const MIGRATION_LOCK_NAME = "breez_mysql_token_store_migration_lock";
const MIGRATION_LOCK_TIMEOUT = 60;

/**
 * Runs a single migration step. Plain strings are run as-is; tagged objects
 * (`{ op: 'dropPrimaryKey', table }`, `{ op: 'dropForeignKey', table, name }`)
 * are guarded against partial-apply replay (and against the `Disabled`
 * foreign-key mode where the FK was never created) by checking
 * `information_schema` first. MySQL DDL implicitly commits, so if the
 * migration crashes between two DDL statements the version row never gets
 * recorded — and on retry, an unguarded DROP would fail because the
 * constraint is already gone.
 */
async function runMigrationStep(conn, step) {
  if (typeof step === "string") {
    await conn.query(step);
    return;
  }
  if (step.op === "dropPrimaryKey") {
    const [rows] = await conn.query(
      `SELECT COUNT(*) AS c FROM information_schema.table_constraints
       WHERE table_schema = DATABASE()
         AND table_name = ?
         AND constraint_type = 'PRIMARY KEY'`,
      [step.table]
    );
    if (rows[0].c > 0) {
      await conn.query(`ALTER TABLE \`${step.table}\` DROP PRIMARY KEY`);
    }
    return;
  }
  if (step.op === "dropForeignKey") {
    const [rows] = await conn.query(
      `SELECT COUNT(*) AS c FROM information_schema.table_constraints
       WHERE table_schema = DATABASE()
         AND table_name = ?
         AND constraint_type = 'FOREIGN KEY'
         AND constraint_name = ?`,
      [step.table, step.name]
    );
    if (rows[0].c > 0) {
      await conn.query(
        `ALTER TABLE \`${step.table}\` DROP FOREIGN KEY \`${step.name}\``
      );
    }
    return;
  }
  if (step.op === "addForeignKey") {
    const [rows] = await conn.query(
      `SELECT COUNT(*) AS c FROM information_schema.table_constraints
       WHERE table_schema = DATABASE()
         AND table_name = ?
         AND constraint_type = 'FOREIGN KEY'
         AND constraint_name = ?`,
      [step.table, step.name]
    );
    if (rows[0].c === 0) {
      await conn.query(
        `ALTER TABLE \`${step.table}\` ADD CONSTRAINT \`${step.name}\` ${step.definition}`
      );
    }
    return;
  }
  throw new Error(`Unknown migration step op: ${JSON.stringify(step)}`);
}

class MysqlTokenStoreMigrationManager {
  constructor(logger = null, foreignKeyMode = "Enforced") {
    this.logger = logger;
    this.foreignKeyMode = foreignKeyMode;
  }

  /**
   * @param {import('mysql2/promise').Pool} pool
   * @param {Buffer|Uint8Array} identity - 33-byte secp256k1 compressed pubkey
   *   identifying the tenant. Used to backfill `user_id` columns in the
   *   multi-tenant scoping migration. Required.
   */
  async migrate(pool, identity) {
    if (!identity || identity.length !== 33) {
      throw new TokenStoreError(
        "tenant identity (33-byte secp256k1 pubkey) is required"
      );
    }
    const conn = await pool.getConnection();
    try {
      const [lockRows] = await conn.query(
        "SELECT GET_LOCK(?, ?) AS acquired",
        [MIGRATION_LOCK_NAME, MIGRATION_LOCK_TIMEOUT]
      );
      if (!lockRows || lockRows[0].acquired !== 1) {
        throw new TokenStoreError(
          `Failed to acquire token store migration lock within ${MIGRATION_LOCK_TIMEOUT}s`
        );
      }

      try {
        await this._applySchemaRenames(conn);

        await conn.query("START TRANSACTION");

        await conn.query(`
          CREATE TABLE IF NOT EXISTS \`${TOKEN_MIGRATIONS_TABLE}\` (
            version INT PRIMARY KEY,
            applied_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))
          )
        `);

        const [versionRows] = await conn.query(
          `SELECT COALESCE(MAX(version), 0) AS version FROM \`${TOKEN_MIGRATIONS_TABLE}\``
        );
        const currentVersion = versionRows[0].version;

        const migrations = this._getMigrations(identity);

        if (currentVersion >= migrations.length) {
          await conn.query("COMMIT");
          return;
        }

        for (let i = currentVersion; i < migrations.length; i++) {
          const migration = migrations[i];
          const version = i + 1;
          for (const step of migration.sql) {
            await runMigrationStep(conn, step);
          }
          await conn.query(
            `INSERT INTO \`${TOKEN_MIGRATIONS_TABLE}\` (version, applied_at) VALUES (?, UTC_TIMESTAMP(6))`,
            [version]
          );
        }

        await conn.query("COMMIT");
      } catch (error) {
        await conn.query("ROLLBACK").catch(() => {});
        throw new TokenStoreError(
          `Token store migration failed: ${error.message}`,
          error
        );
      } finally {
        await conn
          .query("SELECT RELEASE_LOCK(?)", [MIGRATION_LOCK_NAME])
          .catch(() => {});
      }
    } finally {
      conn.release();
    }
  }

  /**
   * @param {Buffer|Uint8Array} identity - tenant identity inlined as a hex
   *   `UNHEX('…')` literal in the multi-tenant scoping migration. Safe because
   *   the bytes come from a typed secp256k1 pubkey (`[0-9a-f]{66}` after hex
   *   encoding) — not user-controlled input.
   */
  /**
   * Pre-prefix rename. Canary-gated on the legacy `token_schema_migrations`
   * table.
   * @param {import('mysql2/promise').PoolConnection} conn
   */
  async _applySchemaRenames(conn) {
    if (!(await _mysqlTableExists(conn, "token_schema_migrations"))) {
      return;
    }

    const tableRenames = [
      ["token_metadata", "brz_token_metadata"],
      ["token_reservations", "brz_token_reservations"],
      ["token_outputs", "brz_token_outputs"],
      ["token_spent_outputs", "brz_token_spent_outputs"],
      ["token_swap_status", "brz_token_swap_status"],
    ];
    for (const [oldName, newName] of tableRenames) {
      if (
        (await _mysqlTableExists(conn, oldName)) &&
        !(await _mysqlTableExists(conn, newName))
      ) {
        await conn.query(`RENAME TABLE \`${oldName}\` TO \`${newName}\``);
      }
    }

    const indexRenames = [
      [
        "brz_token_metadata",
        "idx_token_metadata_user_issuer_pk",
        "brz_idx_token_metadata_user_issuer_pk",
      ],
      [
        "brz_token_outputs",
        "idx_token_outputs_user_identifier",
        "brz_idx_token_outputs_user_identifier",
      ],
      [
        "brz_token_outputs",
        "idx_token_outputs_user_reservation",
        "brz_idx_token_outputs_user_reservation",
      ],
      // Pre-multi-tenant indexes (dropped by the multi-tenant migration).
      [
        "brz_token_metadata",
        "idx_token_metadata_issuer_pk",
        "brz_idx_token_metadata_issuer_pk",
      ],
      [
        "brz_token_outputs",
        "idx_token_outputs_identifier",
        "brz_idx_token_outputs_identifier",
      ],
      [
        "brz_token_outputs",
        "idx_token_outputs_reservation",
        "brz_idx_token_outputs_reservation",
      ],
    ];
    for (const [table, oldName, newName] of indexRenames) {
      if (
        (await _mysqlIndexExists(conn, table, oldName)) &&
        !(await _mysqlIndexExists(conn, table, newName))
      ) {
        await conn.query(
          `ALTER TABLE \`${table}\` RENAME INDEX \`${oldName}\` TO \`${newName}\``
        );
      }
    }

    const fkRenames = [
      {
        table: "brz_token_outputs",
        oldName: "fk_token_outputs_metadata_user",
        newName: "brz_fk_token_outputs_metadata_user",
        definition:
          "FOREIGN KEY (user_id, token_identifier) REFERENCES `brz_token_metadata`(user_id, identifier)",
      },
      {
        table: "brz_token_outputs",
        oldName: "fk_token_outputs_reservation_user",
        newName: "brz_fk_token_outputs_reservation_user",
        definition:
          "FOREIGN KEY (user_id, reservation_id) REFERENCES `brz_token_reservations`(user_id, id)",
      },
      // Pre-multi-tenant FKs (single-column). Rename so the post-tenant
      // migration's drop-foreign-key steps find them.
      {
        table: "brz_token_outputs",
        oldName: "fk_token_outputs_metadata",
        newName: "brz_fk_token_outputs_metadata",
        definition:
          "FOREIGN KEY (token_identifier) REFERENCES `brz_token_metadata`(identifier)",
      },
      {
        table: "brz_token_outputs",
        oldName: "fk_token_outputs_reservation",
        newName: "brz_fk_token_outputs_reservation",
        definition:
          "FOREIGN KEY (reservation_id) REFERENCES `brz_token_reservations`(id) ON DELETE SET NULL",
      },
    ];
    for (const fk of fkRenames) {
      if (await _mysqlForeignKeyExists(conn, fk.table, fk.newName)) {
        continue;
      }
      if (!(await _mysqlForeignKeyExists(conn, fk.table, fk.oldName))) {
        continue;
      }
      await conn.query(
        `ALTER TABLE \`${fk.table}\`` +
          ` DROP FOREIGN KEY \`${fk.oldName}\`,` +
          ` ADD CONSTRAINT \`${fk.newName}\` ${fk.definition}`
      );
    }

    if (
      (await _mysqlTableExists(conn, "token_schema_migrations")) &&
      !(await _mysqlTableExists(conn, TOKEN_MIGRATIONS_TABLE))
    ) {
      await conn.query(
        `RENAME TABLE \`token_schema_migrations\` TO \`${TOKEN_MIGRATIONS_TABLE}\``
      );
    }
  }

  _getMigrations(identity) {
    const idHex = Buffer.from(identity).toString("hex");
    const idLit = `UNHEX('${idHex}')`;
    const foreignKeyModeEnforced = this.foreignKeyMode === "Enforced";

    const initialSql = [
          `CREATE TABLE IF NOT EXISTS brz_token_metadata (
            identifier VARCHAR(255) NOT NULL PRIMARY KEY,
            issuer_public_key VARCHAR(255) NOT NULL,
            name VARCHAR(255) NOT NULL,
            ticker VARCHAR(64) NOT NULL,
            decimals INT NOT NULL,
            max_supply VARCHAR(128) NOT NULL,
            is_freezable TINYINT(1) NOT NULL,
            creation_entity_public_key VARCHAR(255) NULL
          )`,
          `CREATE INDEX brz_idx_token_metadata_issuer_pk
            ON brz_token_metadata (issuer_public_key)`,
          `CREATE TABLE IF NOT EXISTS brz_token_reservations (
            id VARCHAR(255) NOT NULL PRIMARY KEY,
            purpose VARCHAR(64) NOT NULL,
            created_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
          )`,
          `CREATE TABLE IF NOT EXISTS brz_token_outputs (
            id VARCHAR(255) NOT NULL PRIMARY KEY,
            token_identifier VARCHAR(255) NOT NULL,
            owner_public_key VARCHAR(255) NOT NULL,
            revocation_commitment VARCHAR(255) NOT NULL,
            withdraw_bond_sats BIGINT NOT NULL,
            withdraw_relative_block_locktime BIGINT NOT NULL,
            token_public_key VARCHAR(255) NULL,
            token_amount VARCHAR(128) NOT NULL,
            prev_tx_hash VARCHAR(255) NOT NULL,
            prev_tx_vout INT NOT NULL,
            reservation_id VARCHAR(255) NULL,
            added_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
          )`,
          `CREATE INDEX brz_idx_token_outputs_identifier
            ON brz_token_outputs (token_identifier)`,
          `CREATE INDEX brz_idx_token_outputs_reservation
            ON brz_token_outputs (reservation_id)`,
          `CREATE TABLE IF NOT EXISTS brz_token_spent_outputs (
            output_id VARCHAR(255) NOT NULL PRIMARY KEY,
            spent_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
          )`,
          `CREATE TABLE IF NOT EXISTS brz_token_swap_status (
            id INT NOT NULL PRIMARY KEY DEFAULT 1,
            last_completed_at DATETIME(6) NULL,
            CHECK (id = 1)
          )`,
    ];
    if (foreignKeyModeEnforced) {
      initialSql.push(
        {
          op: "addForeignKey",
          table: "brz_token_outputs",
          name: "brz_fk_token_outputs_metadata",
          definition: `FOREIGN KEY (token_identifier) REFERENCES brz_token_metadata(identifier)`,
        },
        {
          op: "addForeignKey",
          table: "brz_token_outputs",
          name: "brz_fk_token_outputs_reservation",
          definition: `FOREIGN KEY (reservation_id) REFERENCES brz_token_reservations(id) ON DELETE SET NULL`,
        }
      );
    }

    return [
      {
        name: "Create token store tables",
        sql: [
          ...initialSql,
          `INSERT INTO brz_token_swap_status (id) VALUES (1)
            ON DUPLICATE KEY UPDATE id = id`,
        ],
      },
      {
        // Mirrors Rust migration 2 in spark-mysql/src/token_store.rs and the
        // postgres equivalent. Adds user_id to every token-store table
        // (including brz_token_metadata — per-tenant to avoid 0-balance leakage
        // for tokens a tenant never owned), backfills with the connecting
        // tenant, and rewrites primary keys / FKs / indexes to lead with
        // user_id. Composite FKs use NO ACTION because column-list SET NULL
        // would null user_id (NOT NULL).
        name: "Multi-tenant scoping: add user_id and rewrite primary keys / FKs",
        sql: [
          // Drop dependent FKs FIRST so we can rewrite the parent PKs.
          // Guarded so that databases created with `Disabled` foreign-key mode
          // (where the FKs were never created) skip the DROP rather than
          // erroring.
          {
            op: "dropForeignKey",
            table: "brz_token_outputs",
            name: "brz_fk_token_outputs_metadata",
          },
          {
            op: "dropForeignKey",
            table: "brz_token_outputs",
            name: "brz_fk_token_outputs_reservation",
          },

          // brz_token_metadata: per-tenant scoping (privacy — see header).
          `ALTER TABLE brz_token_metadata ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE brz_token_metadata SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE brz_token_metadata MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE brz_token_metadata DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, identifier)`,
          `DROP INDEX brz_idx_token_metadata_issuer_pk ON brz_token_metadata`,
          `CREATE INDEX brz_idx_token_metadata_user_issuer_pk
             ON brz_token_metadata (user_id, issuer_public_key)`,

          // brz_token_reservations: scope by user_id.
          `ALTER TABLE brz_token_reservations ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE brz_token_reservations SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE brz_token_reservations MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE brz_token_reservations DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, id)`,

          // brz_token_outputs: scope by user_id, rekey, optionally re-add composite FKs.
          `ALTER TABLE brz_token_outputs ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE brz_token_outputs SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE brz_token_outputs MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE brz_token_outputs DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, id)`,
          ...(foreignKeyModeEnforced
            ? [
                {
                  op: "addForeignKey",
                  table: "brz_token_outputs",
                  name: "brz_fk_token_outputs_metadata_user",
                  definition: `FOREIGN KEY (user_id, token_identifier) REFERENCES brz_token_metadata(user_id, identifier)`,
                },
                {
                  op: "addForeignKey",
                  table: "brz_token_outputs",
                  name: "brz_fk_token_outputs_reservation_user",
                  definition: `FOREIGN KEY (user_id, reservation_id) REFERENCES brz_token_reservations(user_id, id)`,
                },
              ]
            : []),
          `DROP INDEX brz_idx_token_outputs_identifier ON brz_token_outputs`,
          `DROP INDEX brz_idx_token_outputs_reservation ON brz_token_outputs`,
          `CREATE INDEX brz_idx_token_outputs_user_identifier
             ON brz_token_outputs (user_id, token_identifier)`,
          `CREATE INDEX brz_idx_token_outputs_user_reservation
             ON brz_token_outputs (user_id, reservation_id)`,

          // brz_token_spent_outputs: scope by user_id.
          `ALTER TABLE brz_token_spent_outputs ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE brz_token_spent_outputs SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE brz_token_spent_outputs MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE brz_token_spent_outputs DROP PRIMARY KEY, ADD PRIMARY KEY (user_id, output_id)`,

          // brz_token_swap_status was a singleton (PK id=1, CHECK id=1). Drop the
          // PK and the id column, then re-key by user_id.
          { op: "dropPrimaryKey", table: "brz_token_swap_status" },
          `ALTER TABLE brz_token_swap_status DROP COLUMN id`,
          `ALTER TABLE brz_token_swap_status ADD COLUMN user_id VARBINARY(33) NULL`,
          `UPDATE brz_token_swap_status SET user_id = ${idLit} WHERE user_id IS NULL`,
          `ALTER TABLE brz_token_swap_status MODIFY COLUMN user_id VARBINARY(33) NOT NULL`,
          `ALTER TABLE brz_token_swap_status ADD PRIMARY KEY (user_id)`,
        ],
      },
      {
        // Pin DATETIME defaults to UTC. Server-side INSERTs already pass
        // `UTC_TIMESTAMP(6)` explicitly; this migration makes the column
        // default match, so any future callsite that omits the column also
        // gets a UTC value rather than a session-TZ-dependent one.
        name: "Pin DATETIME defaults to UTC",
        sql: [
          `ALTER TABLE brz_token_outputs            MODIFY COLUMN added_at   DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))`,
          `ALTER TABLE brz_token_reservations       MODIFY COLUMN created_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))`,
          `ALTER TABLE brz_token_spent_outputs      MODIFY COLUMN spent_at   DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))`,
          `ALTER TABLE brz_token_schema_migrations  MODIFY COLUMN applied_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))`,
        ],
      },
      {
        // Mirrors Rust migration 4 in spark-mysql/src/token_store.rs.
        // Re-keys brz_token_spent_outputs by (prev_tx_hash, prev_tx_vout) instead
        // of the operator-issued output id. v3 FinalTokenOutput carries no id
        // field, so post-broadcast spent markers only have an outpoint to work
        // with. Existing output_id-keyed rows can't be backfilled (no outpoint
        // stored alongside them), so the table is wiped on upgrade — spent
        // markers are short-lived (5 minute cleanup window) so wiping is
        // equivalent to letting them age out.
        name: "Re-key spent outputs by (prev_tx_hash, prev_tx_vout)",
        sql: [
          `DROP TABLE IF EXISTS brz_token_spent_outputs`,
          `CREATE TABLE brz_token_spent_outputs (
             user_id VARBINARY(33) NOT NULL,
             prev_tx_hash VARCHAR(255) NOT NULL,
             prev_tx_vout INT NOT NULL,
             spent_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6)),
             PRIMARY KEY (user_id, prev_tx_hash, prev_tx_vout)
           )`,
        ],
      },
      {
        // Mirrors Rust migration 5 in spark-mysql/src/token_store.rs.
        // Re-key brz_token_outputs by (prev_tx_hash, prev_tx_vout) and drop the
        // legacy id column. id already held "{prev_tx_hash}:{vout}", so the
        // outpoint is the natural key. Dedup any duplicate-outpoint rows
        // (possible from pre-outpoint code) before adding the composite PK,
        // preferring rows that hold a reservation.
        name: "Re-key token outputs by (prev_tx_hash, prev_tx_vout), drop legacy id",
        sql: [
          `DELETE a FROM brz_token_outputs a
           JOIN brz_token_outputs b
             ON a.user_id = b.user_id
            AND a.prev_tx_hash = b.prev_tx_hash
            AND a.prev_tx_vout = b.prev_tx_vout
            AND ((b.reservation_id IS NOT NULL) > (a.reservation_id IS NOT NULL)
                 OR ((b.reservation_id IS NOT NULL) = (a.reservation_id IS NOT NULL)
                     AND b.id > a.id))`,
          `ALTER TABLE brz_token_outputs
             DROP PRIMARY KEY,
             ADD PRIMARY KEY (user_id, prev_tx_hash, prev_tx_vout)`,
          `ALTER TABLE brz_token_outputs DROP COLUMN id`,
        ],
      },
    ];
  }
}

async function _mysqlTableExists(conn, tableName) {
  const [rows] = await conn.query(
    `SELECT COUNT(*) AS c FROM information_schema.tables
     WHERE table_schema = DATABASE() AND table_name = ?`,
    [tableName]
  );
  return Number(rows[0].c) > 0;
}

async function _mysqlIndexExists(conn, tableName, indexName) {
  const [rows] = await conn.query(
    `SELECT COUNT(*) AS c FROM information_schema.statistics
     WHERE table_schema = DATABASE() AND table_name = ? AND index_name = ?`,
    [tableName, indexName]
  );
  return Number(rows[0].c) > 0;
}

async function _mysqlForeignKeyExists(conn, tableName, constraintName) {
  const [rows] = await conn.query(
    `SELECT COUNT(*) AS c FROM information_schema.table_constraints
     WHERE table_schema = DATABASE()
       AND table_name = ?
       AND constraint_type = 'FOREIGN KEY'
       AND constraint_name = ?`,
    [tableName, constraintName]
  );
  return Number(rows[0].c) > 0;
}

module.exports = { MysqlTokenStoreMigrationManager };
