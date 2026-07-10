use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::repository::{DomainConfig, LnurlRepository, LnurlRepositoryError};

const REFRESH_INTERVAL: Duration = Duration::from_mins(1);

/// Allowed domains mapped to their own Breez API key, or `None` when the domain
/// has none and falls back to the configured default. The api key is exchanged
/// for the partner JWT that carries Spark attribution.
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
    let (initial, without_api_key) = load(&db).await?;
    log_loaded(&initial);
    if warn_missing_api_keys {
        warn_missing_api_key(&without_api_key, default_api_key.is_some());
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

/// Load the own-key map together with the domains that have no key of their own.
async fn load<DB>(db: &DB) -> Result<(DomainMap, Vec<String>), LnurlRepositoryError>
where
    DB: LnurlRepository,
{
    let configs = db.list_domains().await?;
    let without_api_key = domains_without_api_key(&configs);
    Ok((to_map(configs), without_api_key))
}

/// Build the domain -> own-key map (`None` for a domain with no key of its own).
fn to_map(configs: Vec<DomainConfig>) -> DomainMap {
    configs.into_iter().map(|d| (d.domain, d.api_key)).collect()
}

/// Domains that have no Breez API key of their own.
fn domains_without_api_key(configs: &[DomainConfig]) -> Vec<String> {
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
fn warn_missing_api_key(without_api_key: &[String], has_default: bool) {
    for domain in without_api_key {
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
    let (latest, without_api_key) = match load(db).await {
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
        warn_missing_api_key(&without_api_key, default_api_key.is_some());
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
    fn to_map_keeps_own_key() {
        let configs = vec![cfg("a.com", Some("own")), cfg("b.com", None)];
        let map = to_map(configs);
        assert_eq!(map.get("a.com"), Some(&Some("own".to_string())));
        // A domain with no api key maps to None; the default is applied when
        // selecting the wallet, not here.
        assert_eq!(map.get("b.com"), Some(&None));
    }

    #[test]
    fn lists_domains_without_an_api_key() {
        // Independent of any default: the warning tracks own keys.
        let configs = vec![cfg("a.com", Some("own")), cfg("b.com", None)];
        assert_eq!(domains_without_api_key(&configs), vec!["b.com".to_string()]);
    }
}
