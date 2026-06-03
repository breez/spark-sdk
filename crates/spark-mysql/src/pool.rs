//! `MySQL` connection pool creation.
//!
//! Built on top of `mysql_async::Pool`, which provides its own async pool with
//! min/max constraints and idle TTL. We translate `MysqlStorageConfig` knobs
//! onto the closest `mysql_async` equivalents.

use std::time::Duration;

use mysql_async::{
    IsolationLevel, Opts, OptsBuilder, Pool, PoolConstraints, PoolOpts, SslOpts, TxOpts,
};

use crate::config::MysqlStorageConfig;
use crate::error::MysqlError;

/// `TxOpts` pinned to `READ COMMITTED` isolation. `InnoDB`'s default `REPEATABLE READ`
/// applies next-key/gap locks across scanned ranges and even across non-existent
/// rows in `IN`-list lookups, which expands every transaction's lock footprint
/// far beyond the rows it actually touches and lets unrelated writers cycle
/// through deadlock detection. `READ COMMITTED` keeps locks scoped to rows that
/// match — matching the semantics the postgres backend already runs under, and
/// what the per-tenant advisory `GET_LOCK` already assumes when it serializes
/// writers.
pub fn tx_opts() -> TxOpts {
    let mut opts = TxOpts::default();
    opts.with_isolation_level(IsolationLevel::ReadCommitted);
    opts
}

/// Creates a `MySQL` connection pool from the given configuration.
///
/// Honors the `ssl-mode` URL parameter for TLS:
/// - `disabled` — no TLS
/// - `preferred` / `required` — TLS without certificate verification
/// - `verify_ca` / `verify_identity` — TLS with the CA from `root_ca_pem`
///   (or system roots if not provided)
pub fn create_pool(config: &MysqlStorageConfig) -> Result<Pool, MysqlError> {
    let (connection_string, ssl_mode) = connection_string_and_ssl_mode(&config.connection_string);
    let opts: Opts = Opts::from_url(&connection_string)
        .map_err(|e| MysqlError::Initialization(format!("Invalid connection string: {e}")))?;

    let mut builder = with_tcp_keepalive_default(opts);

    if let Some(ssl_opts) = build_ssl_opts(ssl_mode, config.root_ca_pem.as_deref()) {
        builder = builder.ssl_opts(ssl_opts);
    }

    let max = std::cmp::max(config.max_pool_size, 1) as usize;
    let constraints =
        PoolConstraints::new(0, max).unwrap_or_else(|| PoolConstraints::new(0, 10).unwrap());
    let mut pool_opts = PoolOpts::default().with_constraints(constraints);

    if let Some(secs) = config.recycle_timeout_secs {
        pool_opts = pool_opts.with_inactive_connection_ttl(Duration::from_secs(secs));
    }

    builder = builder.pool_opts(pool_opts);

    Ok(Pool::new(builder))
}

/// Sets a 60s TCP keepalive unless the connection string already specified one.
///
/// Without keepalives a managed-`MySQL` NAT/load balancer silently reaps an idle
/// connection; the pool hands the half-open socket back (it pings only in tests,
/// not on checkout) and the next query hangs ~15 min on the dead socket. Probing
/// every 60s keeps the NAT mapping warm. `mysql_async` exposes no probe
/// interval/retries or `tcp_user_timeout`, so idle time is the only knob here.
fn with_tcp_keepalive_default(opts: Opts) -> OptsBuilder {
    let keepalive_set = opts.tcp_keepalive().is_some();
    let mut builder = OptsBuilder::from_opts(opts);
    if !keepalive_set {
        builder = builder.tcp_keepalive(Some(60_000u32));
    }
    builder
}

/// Parses an `ssl-mode` value from a `MySQL` URL connection string and constructs
/// matching `SslOpts`.
fn build_ssl_opts(ssl_mode: SslModeExt, root_ca_pem: Option<&str>) -> Option<SslOpts> {
    match ssl_mode {
        SslModeExt::Disabled => None,
        SslModeExt::Preferred | SslModeExt::Required => {
            // Encryption without identity verification.
            Some(SslOpts::default().with_danger_accept_invalid_certs(true))
        }
        SslModeExt::VerifyCa => {
            let mut opts = SslOpts::default().with_danger_skip_domain_validation(true);
            if let Some(pem) = root_ca_pem {
                opts = opts.with_root_certs(vec![pem.as_bytes().to_vec().into()]);
            }
            Some(opts)
        }
        SslModeExt::VerifyIdentity => {
            let mut opts = SslOpts::default();
            if let Some(pem) = root_ca_pem {
                opts = opts.with_root_certs(vec![pem.as_bytes().to_vec().into()]);
            }
            Some(opts)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SslModeExt {
    Disabled,
    Preferred,
    Required,
    VerifyCa,
    VerifyIdentity,
}

fn connection_string_and_ssl_mode(conn_str: &str) -> (String, SslModeExt) {
    let Some((base, query)) = conn_str.split_once('?') else {
        // Default for MySQL clients is preferred when supported, but the safe
        // default for an unspecified backend is no TLS to avoid surprising
        // failures on local docker setups.
        return (conn_str.to_string(), SslModeExt::Disabled);
    };

    let mut ssl_mode = SslModeExt::Disabled;
    let mut retained_params = Vec::new();

    for param in query.split('&') {
        if let Some((key, value)) = param.split_once('=') {
            let key_lc = key.to_ascii_lowercase();
            if key_lc == "ssl-mode" || key_lc == "ssl_mode" || key_lc == "sslmode" {
                ssl_mode = parse_ssl_mode_value(value);
                continue;
            }
        }
        retained_params.push(param);
    }

    let connection_string = if retained_params.is_empty() {
        base.to_string()
    } else {
        format!("{}?{}", base, retained_params.join("&"))
    };

    (connection_string, ssl_mode)
}

#[allow(clippy::match_same_arms)] // explicit "disabled" arm + unknown-fallback arm both default to Disabled
fn parse_ssl_mode_value(value: &str) -> SslModeExt {
    match value.to_ascii_lowercase().as_str() {
        "disabled" | "disable" => SslModeExt::Disabled,
        "preferred" | "prefer" => SslModeExt::Preferred,
        "required" | "require" => SslModeExt::Required,
        "verify_ca" | "verify-ca" | "verifyca" => SslModeExt::VerifyCa,
        "verify_identity" | "verify-identity" | "verifyidentity" | "verify-full"
        | "verify_full" => SslModeExt::VerifyIdentity,
        _ => SslModeExt::Disabled,
    }
}

/// Maps a `mysql_async` error to `MysqlError`.
///
/// IO errors and connection-class server errors are mapped to `Connection`,
/// other errors to `Database`.
#[allow(clippy::needless_pass_by_value)]
pub fn map_db_error(e: mysql_async::Error) -> MysqlError {
    use mysql_async::Error;
    match e {
        Error::Io(_) => MysqlError::Connection(e.to_string()),
        Error::Server(ref err) => {
            // MySQL server error codes for connection-class issues:
            // 1040: Too many connections
            // 1042: Can't get hostname
            // 1043: Bad handshake
            // 1047: Unknown command
            // 1053: Server shutdown in progress
            // 1077: Got a packet bigger than 'max_allowed_packet' bytes
            // 1158/1159/1160/1161: Network errors
            // 2002/2003/2006/2013: Client-reported connection errors
            // CR_SERVER_GONE_ERROR (2006), CR_SERVER_LOST (2013)
            match err.code {
                1040 | 1043 | 1053 | 1077 | 1158..=1161 | 2002 | 2003 | 2006 | 2013 => {
                    MysqlError::Connection(e.to_string())
                }
                _ => MysqlError::Database(e.to_string()),
            }
        }
        _ => MysqlError::Database(e.to_string()),
    }
}

impl From<mysql_async::Error> for MysqlError {
    fn from(value: mysql_async::Error) -> Self {
        map_db_error(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keepalive_default_applied_when_url_silent() {
        let opts = Opts::from_url("mysql://u:p@h:3306/db").expect("valid");
        let built = Opts::from(with_tcp_keepalive_default(opts));
        assert_eq!(built.tcp_keepalive(), Some(60_000));
    }

    #[test]
    fn keepalive_default_respects_explicit_url_value() {
        let opts = Opts::from_url("mysql://u:p@h:3306/db?tcp_keepalive=5000").expect("valid");
        let built = Opts::from(with_tcp_keepalive_default(opts));
        assert_eq!(built.tcp_keepalive(), Some(5000));
    }

    #[test]
    fn extracts_ssl_mode_before_mysql_async_url_parsing() {
        assert_eq!(
            connection_string_and_ssl_mode("mysql://u:p@host:3306/db?ssl-mode=required"),
            ("mysql://u:p@host:3306/db".to_string(), SslModeExt::Required)
        );
        assert_eq!(
            connection_string_and_ssl_mode(
                "mysql://u:p@host:3306/db?stmt_cache_size=100&ssl_mode=verify_ca&pool_max=4"
            ),
            (
                "mysql://u:p@host:3306/db?stmt_cache_size=100&pool_max=4".to_string(),
                SslModeExt::VerifyCa,
            )
        );
        assert_eq!(
            connection_string_and_ssl_mode("mysql://u:p@host:3306/db?sslmode=disabled&pool_min=1"),
            (
                "mysql://u:p@host:3306/db?pool_min=1".to_string(),
                SslModeExt::Disabled,
            )
        );
    }
}
