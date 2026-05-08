//! Shared storage helpers for Spark SQL backends.

use std::error::Error;
use std::fmt::{Display, Formatter};

const STORAGE_IDENTIFIERS: &[&str] = &[
    // Migration tables.
    "schema_migrations",
    "tree_schema_migrations",
    "token_schema_migrations",
    // Core SDK storage tables.
    "payments",
    "settings",
    "unclaimed_deposits",
    "payment_metadata",
    "payment_details_lightning",
    "payment_details_token",
    "payment_details_spark",
    "lnurl_receive_metadata",
    "sync_revision",
    "sync_outgoing",
    "sync_state",
    "sync_incoming",
    "contacts",
    // Tree store tables.
    "tree_reservations",
    "tree_leaves",
    "tree_spent_leaves",
    "tree_swap_status",
    // Token store tables.
    "token_metadata",
    "token_reservations",
    "token_outputs",
    "token_spent_outputs",
    "token_swap_status",
    // Core SDK storage indexes.
    "idx_payments_timestamp",
    "idx_payments_payment_type",
    "idx_payments_status",
    "idx_payment_details_lightning_invoice",
    "idx_payment_metadata_parent",
    "idx_sync_outgoing_data_id_record_type",
    "idx_sync_incoming_revision",
    "idx_payment_details_lightning_payment_hash",
    "idx_payments_user_timestamp",
    "idx_payments_user_payment_type",
    "idx_payments_user_status",
    "idx_payment_metadata_user_parent",
    "idx_payment_details_lightning_user_invoice",
    "idx_payment_details_lightning_user_payment_hash",
    "idx_sync_outgoing_user_record_type_data_id",
    "idx_sync_incoming_user_revision",
    // Core SDK storage PostgreSQL default primary key names.
    "payments_pkey",
    "settings_pkey",
    "unclaimed_deposits_pkey",
    "payment_metadata_pkey",
    "payment_details_lightning_pkey",
    "payment_details_token_pkey",
    "payment_details_spark_pkey",
    "lnurl_receive_metadata_pkey",
    "sync_revision_pkey",
    "sync_state_pkey",
    "sync_incoming_pkey",
    "contacts_pkey",
    // Tree store indexes.
    "idx_tree_leaves_available",
    "idx_tree_leaves_reservation",
    "idx_tree_leaves_added_at",
    "idx_tree_leaves_slim",
    "idx_tree_leaves_user_available",
    "idx_tree_leaves_user_reservation",
    "idx_tree_leaves_user_added_at",
    "idx_tree_leaves_user_slim",
    // Token store indexes.
    "idx_token_metadata_issuer_pk",
    "idx_token_outputs_identifier",
    "idx_token_outputs_reservation",
    "idx_token_metadata_user_issuer_pk",
    "idx_token_outputs_user_identifier",
    "idx_token_outputs_user_reservation",
    // MySQL explicitly named foreign keys.
    "fk_tree_leaves_reservation",
    "fk_tree_leaves_reservation_user",
    "fk_token_outputs_metadata",
    "fk_token_outputs_metadata_user",
    "fk_token_outputs_reservation",
    "fk_token_outputs_reservation_user",
    // PostgreSQL default constraint names used by migrations.
    "tree_reservations_pkey",
    "tree_leaves_pkey",
    "tree_leaves_reservation_id_fkey",
    "tree_spent_leaves_pkey",
    "token_metadata_pkey",
    "token_reservations_pkey",
    "token_outputs_pkey",
    "token_outputs_token_identifier_fkey",
    "token_outputs_reservation_id_fkey",
    "token_spent_outputs_pkey",
];

/// Invalid SQL table-prefix configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidTablePrefix {
    message: String,
}

impl InvalidTablePrefix {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for InvalidTablePrefix {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for InvalidTablePrefix {}

/// Prefix-aware SQL identifier rewriter for SDK-owned storage identifiers.
#[derive(Clone, Debug, Default)]
pub struct TableNameRewriter {
    prefix: String,
}

impl TableNameRewriter {
    /// Creates a rewriter from an optional table prefix.
    pub fn new(prefix: Option<&str>) -> Result<Self, InvalidTablePrefix> {
        let prefix = prefix.unwrap_or_default();
        validate_table_prefix(prefix)?;
        Ok(Self {
            prefix: prefix.to_string(),
        })
    }

    /// Creates a rewriter with no table prefix.
    #[must_use]
    pub fn unprefixed() -> Self {
        Self::default()
    }

    /// Returns the configured prefix, if any.
    #[must_use]
    pub fn prefix(&self) -> Option<&str> {
        if self.prefix.is_empty() {
            None
        } else {
            Some(self.prefix.as_str())
        }
    }

    /// Applies the configured prefix to a known SDK storage identifier.
    #[must_use]
    pub fn identifier(&self, identifier: &str) -> String {
        self.prefixed_identifier(identifier)
            .unwrap_or_else(|| identifier.to_string())
    }

    /// Applies the configured prefix to known SDK storage identifiers in SQL.
    ///
    /// The rewriter is intentionally narrow: it only rewrites exact identifiers
    /// from the SDK storage allowlist and skips string literals, so JSON paths
    /// and user data are left alone.
    #[must_use]
    pub fn sql(&self, sql: &str) -> String {
        if self.prefix.is_empty() {
            return sql.to_string();
        }

        let bytes = sql.as_bytes();
        let mut out = String::with_capacity(sql.len());
        let mut i = 0;

        while i < bytes.len() {
            match bytes[i] {
                b'\'' => {
                    i = copy_quoted_literal(sql, &mut out, i, bytes[i]);
                }
                b'`' | b'"' => {
                    i = copy_quoted_identifier(self, sql, &mut out, i, bytes[i]);
                }
                b if is_identifier_start(b) => {
                    let start = i;
                    i += 1;
                    while i < bytes.len() && is_identifier_part(bytes[i]) {
                        i += 1;
                    }
                    let identifier = &sql[start..i];
                    if let Some(prefixed) = self.prefixed_identifier(identifier) {
                        out.push_str(&prefixed);
                    } else {
                        out.push_str(identifier);
                    }
                }
                b => {
                    out.push(char::from(b));
                    i += 1;
                }
            }
        }

        out
    }

    fn prefixed_identifier(&self, identifier: &str) -> Option<String> {
        if self.prefix.is_empty() {
            return None;
        }
        if is_known_identifier(identifier) {
            return Some(format!("{}{}", self.prefix, identifier));
        }
        identifier
            .strip_prefix(&self.prefix)
            .and_then(|unprefixed| is_known_identifier(unprefixed).then(|| identifier.to_string()))
    }
}

/// Validates a SQL table prefix.
pub fn validate_table_prefix(prefix: &str) -> Result<(), InvalidTablePrefix> {
    if prefix.is_empty() {
        return Ok(());
    }

    let mut chars = prefix.chars();
    let first = chars.next().expect("non-empty prefix has first char");
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(InvalidTablePrefix::new(
            "table_prefix must start with an ASCII letter or underscore",
        ));
    }

    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(InvalidTablePrefix::new(
            "table_prefix may only contain ASCII letters, digits, and underscores",
        ));
    }

    Ok(())
}

fn is_known_identifier(identifier: &str) -> bool {
    STORAGE_IDENTIFIERS.contains(&identifier)
}

fn is_identifier_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_identifier_part(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn copy_quoted_literal(sql: &str, out: &mut String, start: usize, quote: u8) -> usize {
    let bytes = sql.as_bytes();
    let mut i = start;
    out.push(char::from(quote));
    i += 1;

    while i < bytes.len() {
        let b = bytes[i];
        out.push(char::from(b));
        i += 1;

        if b == b'\\' && i < bytes.len() {
            out.push(char::from(bytes[i]));
            i += 1;
            continue;
        }

        if b == quote {
            if i < bytes.len() && bytes[i] == quote {
                out.push(char::from(bytes[i]));
                i += 1;
                continue;
            }
            break;
        }
    }

    i
}

fn copy_quoted_identifier(
    table_names: &TableNameRewriter,
    sql: &str,
    out: &mut String,
    start: usize,
    quote: u8,
) -> usize {
    let bytes = sql.as_bytes();
    let mut identifier = String::new();
    let mut i = start + 1;

    while i < bytes.len() {
        let b = bytes[i];
        if b == quote {
            if i + 1 < bytes.len() && bytes[i + 1] == quote {
                identifier.push(char::from(quote));
                i += 2;
                continue;
            }

            out.push(char::from(quote));
            push_escaped_identifier(out, &table_names.identifier(&identifier), quote);
            out.push(char::from(quote));
            return i + 1;
        }

        identifier.push(char::from(b));
        i += 1;
    }

    out.push(char::from(quote));
    push_escaped_identifier(out, &identifier, quote);
    i
}

fn push_escaped_identifier(out: &mut String, identifier: &str, quote: u8) {
    let quote = char::from(quote);
    for c in identifier.chars() {
        out.push(c);
        if c == quote {
            out.push(c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_known_table_names_outside_literals() {
        let names = TableNameRewriter::new(Some("breez_")).unwrap();

        let sql = names.sql(
            "INSERT INTO payments (id) SELECT id FROM `settings` WHERE value = 'payments tree_leaves'",
        );

        assert_eq!(
            sql,
            "INSERT INTO breez_payments (id) SELECT id FROM `breez_settings` WHERE value = 'payments tree_leaves'"
        );
    }

    #[test]
    fn prefixes_indexes_and_constraints() {
        let names = TableNameRewriter::new(Some("breez_")).unwrap();

        assert_eq!(
            names
                .sql("CREATE INDEX idx_tree_leaves_user_available ON tree_leaves(user_id, status)"),
            "CREATE INDEX breez_idx_tree_leaves_user_available ON breez_tree_leaves(user_id, status)"
        );
        assert_eq!(
            names.sql(
                "ALTER TABLE payment_details_lightning DROP CONSTRAINT IF EXISTS payment_details_lightning_pkey"
            ),
            "ALTER TABLE breez_payment_details_lightning DROP CONSTRAINT IF EXISTS breez_payment_details_lightning_pkey"
        );
        assert_eq!(
            names.sql("ALTER TABLE \"tree_leaves\" DROP CONSTRAINT IF EXISTS \"tree_leaves_pkey\""),
            "ALTER TABLE \"breez_tree_leaves\" DROP CONSTRAINT IF EXISTS \"breez_tree_leaves_pkey\""
        );
    }

    #[test]
    fn rejects_prefixes_that_are_not_safe_unquoted_identifiers() {
        assert!(TableNameRewriter::new(Some("breez_")).is_ok());
        assert!(TableNameRewriter::new(Some("_breez_")).is_ok());
        assert!(TableNameRewriter::new(Some("1breez_")).is_err());
        assert!(TableNameRewriter::new(Some("breez-")).is_err());
    }
}
