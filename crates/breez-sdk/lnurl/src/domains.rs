use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::repository::{DomainConfig, LnurlRepository, LnurlRepositoryError};

const REFRESH_INTERVAL: Duration = Duration::from_mins(1);

/// Allowed domains mapped to their effective Breez API key: the domain's own
/// key, or the configured default when it has none (exchanged for the partner
/// JWT that carries Spark attribution). `None` only when neither is set.
pub type DomainMap = HashMap<String, Option<String>>;

/// Load the allowed domains from the database and start a background task that
/// periodically refreshes them. Returns the shared handle that should be stored
/// on `State` so request handlers and the partner JWT provider observe updates.
pub async fn start<DB>(
    db: DB,
    warn_missing_api_keys: bool,
    default_api_key: Option<String>,
) -> Result<Arc<RwLock<DomainMap>>, LnurlRepositoryError>
where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    let (initial, keyless) = load(&db, default_api_key.as_deref()).await?;
    log_loaded(&initial);
    if warn_missing_api_keys {
        warn_keyless(&keyless, default_api_key.is_some());
    }
    let domains = Arc::new(RwLock::new(initial));

    let db_clone = db.clone();
    let domains_clone = Arc::clone(&domains);
    tokio::spawn(async move {
        debug!("Allowed domains refresher started");
        loop {
            tokio::time::sleep(REFRESH_INTERVAL).await;
            refresh_once(
                &db_clone,
                &domains_clone,
                warn_missing_api_keys,
                default_api_key.as_deref(),
            )
            .await;
        }
    });

    Ok(domains)
}

/// Load the effective-key map together with the domains that have no key of
/// their own (surfaced by the keyless warning, whether or not a default covers
/// them).
async fn load<DB>(
    db: &DB,
    default_api_key: Option<&str>,
) -> Result<(DomainMap, Vec<String>), LnurlRepositoryError>
where
    DB: LnurlRepository,
{
    let configs = db.list_domains().await?;
    let keyless = keyless_domains(&configs);
    Ok((to_map(configs, default_api_key), keyless))
}

/// Build the domain -> effective-key map, falling back to `default_api_key` for any
/// domain without its own.
fn to_map(configs: Vec<DomainConfig>, default_api_key: Option<&str>) -> DomainMap {
    configs
        .into_iter()
        .map(|d| {
            (
                d.domain,
                d.api_key.or_else(|| default_api_key.map(String::from)),
            )
        })
        .collect()
}

/// Domains that have no Breez API key of their own.
fn keyless_domains(configs: &[DomainConfig]) -> Vec<String> {
    configs
        .iter()
        .filter(|d| d.api_key.is_none())
        .map(|d| d.domain.clone())
        .collect()
}

fn log_loaded(map: &DomainMap) {
    let mut sorted: Vec<&str> = map.keys().map(String::as_str).collect();
    sorted.sort_unstable();
    info!("loaded allowed domains: {}", sorted.join(", "));
}

/// Warn about allowed domains without a Breez API key of their own. With a
/// default configured they are still attributed (to the default); without one
/// their lightning-address receives are recorded unattributed.
fn warn_keyless(keyless: &[String], has_default: bool) {
    for domain in keyless {
        if has_default {
            warn!(
                "allowed domain '{domain}' has no Breez API key of its own; using the default key"
            );
        } else {
            warn!(
                "allowed domain '{domain}' has no Breez API key; its lightning-address receives will be unattributed"
            );
        }
    }
}

async fn refresh_once<DB>(
    db: &DB,
    domains: &RwLock<DomainMap>,
    warn_missing_api_keys: bool,
    default_api_key: Option<&str>,
) where
    DB: LnurlRepository,
{
    let (latest, keyless) = match load(db, default_api_key).await {
        Ok(loaded) => loaded,
        Err(e) => {
            error!("Failed to refresh allowed domains: {}", e);
            return;
        }
    };

    {
        let current = domains.read().await;
        if *current == latest {
            return;
        }
        log_changes(&current, &latest);
    }

    if warn_missing_api_keys {
        warn_keyless(&keyless, default_api_key.is_some());
    }
    *domains.write().await = latest;
}

/// Log added/removed domains and api-key rotations so both are observable.
fn log_changes(current: &DomainMap, latest: &DomainMap) {
    for domain in latest.keys() {
        match current.get(domain) {
            None => info!("allowed domain added: {}", domain),
            Some(old) if old != &latest[domain] => {
                info!("Breez API key changed for domain: {}", domain);
            }
            Some(_) => {}
        }
    }
    for domain in current.keys() {
        if !latest.contains_key(domain) {
            info!("allowed domain removed: {}", domain);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(domain: &str, api_key: Option<&str>) -> DomainConfig {
        DomainConfig {
            domain: domain.to_string(),
            api_key: api_key.map(str::to_string),
            jwt: None,
        }
    }

    #[test]
    fn to_map_falls_back_to_default_api_key() {
        let configs = vec![cfg("a.com", Some("own")), cfg("b.com", None)];

        // With a default, a keyless domain inherits it; a keyed one keeps its own.
        let map = to_map(configs.clone(), Some("default"));
        assert_eq!(map.get("a.com"), Some(&Some("own".to_string())));
        assert_eq!(map.get("b.com"), Some(&Some("default".to_string())));

        // Without a default, a keyless domain stays unattributed (None).
        let map = to_map(configs, None);
        assert_eq!(map.get("a.com"), Some(&Some("own".to_string())));
        assert_eq!(map.get("b.com"), Some(&None));
    }

    #[test]
    fn keyless_domains_lists_those_without_own_key() {
        // Independent of any default: the warning tracks own keys.
        let configs = vec![cfg("a.com", Some("own")), cfg("b.com", None)];
        assert_eq!(keyless_domains(&configs), vec!["b.com".to_string()]);
    }
}
