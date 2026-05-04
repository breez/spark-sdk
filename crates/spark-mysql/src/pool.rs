//! `MySQL` connection pool creation.
//!
//! Built on top of `mysql_async::Pool`, which provides its own async pool with
//! min/max constraints and idle TTL. We translate `MysqlStorageConfig` knobs
//! onto the closest `mysql_async` equivalents.

use std::time::Duration;

use mysql_async::{Opts, OptsBuilder, Pool, PoolConstraints, PoolOpts, SslOpts};

use crate::config::MysqlStorageConfig;
use crate::error::MysqlError;

/// Creates a `MySQL` connection pool from the given configuration.
///
/// Honors the `ssl-mode` URL parameter for TLS:
/// - `disabled` — no TLS
/// - `preferred` / `required` — TLS without certificate verification
/// - `verify_ca` / `verify_identity` — TLS with the CA from `root_ca_pem`
///   (or system roots if not provided)
pub fn create_pool(config: &MysqlStorageConfig) -> Result<Pool, MysqlError> {
    let opts: Opts = Opts::from_url(&config.connection_string)
        .map_err(|e| MysqlError::Initialization(format!("Invalid connection string: {e}")))?;

    let mut builder = OptsBuilder::from_opts(opts);

    if let Some(ssl_opts) =
        build_ssl_opts(&config.connection_string, config.root_ca_pem.as_deref())?
    {
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

    // Note: mysql_async's Pool doesn't expose explicit wait/create timeouts at the pool layer.
    // We accept those fields for API parity with the postgres config; honoring them would
    // require wrapping every pool.get_conn() call with tokio::time::timeout, which we can
    // add later if a deployment needs it.
    let _ = config.wait_timeout_secs;
    let _ = config.create_timeout_secs;

    Ok(Pool::new(builder))
}

/// Parses an `ssl-mode` value from a `MySQL` URL connection string and constructs
/// matching `SslOpts`.
#[allow(clippy::unnecessary_wraps)] // future-proofs for cert parsing errors
fn build_ssl_opts(
    conn_str: &str,
    root_ca_pem: Option<&str>,
) -> Result<Option<SslOpts>, MysqlError> {
    let ssl_mode = parse_ssl_mode_from_url(conn_str);
    match ssl_mode {
        SslModeExt::Disabled => Ok(None),
        SslModeExt::Preferred | SslModeExt::Required => {
            // Encryption without identity verification.
            Ok(Some(
                SslOpts::default().with_danger_accept_invalid_certs(true),
            ))
        }
        SslModeExt::VerifyCa => {
            let mut opts = SslOpts::default().with_danger_skip_domain_validation(true);
            if let Some(pem) = root_ca_pem {
                opts = opts.with_root_certs(vec![pem.as_bytes().to_vec().into()]);
            }
            Ok(Some(opts))
        }
        SslModeExt::VerifyIdentity => {
            let mut opts = SslOpts::default();
            if let Some(pem) = root_ca_pem {
                opts = opts.with_root_certs(vec![pem.as_bytes().to_vec().into()]);
            }
            Ok(Some(opts))
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

fn parse_ssl_mode_from_url(conn_str: &str) -> SslModeExt {
    let Some(query) = conn_str.split_once('?').map(|(_, q)| q) else {
        // Default for MySQL clients is preferred when supported, but the safe
        // default for an unspecified backend is no TLS to avoid surprising
        // failures on local docker setups.
        return SslModeExt::Disabled;
    };

    for param in query.split('&') {
        if let Some((key, value)) = param.split_once('=') {
            let key_lc = key.to_ascii_lowercase();
            if key_lc == "ssl-mode" || key_lc == "ssl_mode" || key_lc == "sslmode" {
                return parse_ssl_mode_value(value);
            }
        }
    }
    SslModeExt::Disabled
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
