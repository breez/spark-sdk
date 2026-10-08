use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
    time::Duration,
};

use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::repository::{DomainConfig, LnurlRepository, LnurlRepositoryError};

const REFRESH_INTERVAL: Duration = Duration::from_mins(1);

/// Every allowed domain, by name.
pub type DomainMap = HashMap<String, Domain>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Domain {
    /// The group the domain is in. `None` when the server refuses the group
    /// the domain is configured with, and so does not serve the domain.
    pub group: Option<DomainGroup>,
    /// The domain's own Breez API key, exchanged for the partner JWT that
    /// carries Spark attribution. `None` when the domain is in another domain's
    /// group, whose key applies instead, or when it has no key and falls back
    /// to the configured default.
    pub api_key: Option<String>,
}

/// A group of domains that serve the same users, named after one of them.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DomainGroup(String);

impl DomainGroup {
    pub fn new(domain: impl Into<String>) -> Self {
        Self(domain.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DomainGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The group `domain` is in, or `None` when the server does not serve `domain`.
pub fn group_of(domains: &DomainMap, domain: &str, is_mainnet: bool) -> Option<DomainGroup> {
    match domains.get(domain) {
        Some(allowed) => allowed.group.clone(),
        // An empty allow-list falls open to any host, for local/test setups only.
        // Never on mainnet: there it must be an explicit deny.
        None if domains.is_empty() && !is_mainnet => Some(DomainGroup::new(domain)),
        None => None,
    }
}

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

/// Load the allowed domains together with the ones that have no key of their own.
async fn load<DB>(db: &DB) -> Result<(DomainMap, Vec<String>), LnurlRepositoryError>
where
    DB: LnurlRepository,
{
    let configs = db.list_domains().await?;
    let without_api_key = domains_without_api_key(&configs);
    Ok((to_map(configs), without_api_key))
}

fn to_map(configs: Vec<DomainConfig>) -> DomainMap {
    let group_names: HashSet<String> = configs
        .iter()
        .filter(|config| joined_group(config).is_none())
        .map(|config| config.domain.clone())
        .collect();
    configs
        .into_iter()
        .map(|config| {
            let domain = resolve(&config, &group_names);
            (config.domain, domain)
        })
        .collect()
}

/// The other domain whose group `config` is in, if it is not in its own.
fn joined_group(config: &DomainConfig) -> Option<&str> {
    config
        .domain_group
        .as_deref()
        .filter(|group| *group != config.domain)
}

/// How the server serves `config`. A group can be named after any of
/// `group_names`.
fn resolve(config: &DomainConfig, group_names: &HashSet<String>) -> Domain {
    let Some(group) = joined_group(config) else {
        return Domain {
            group: Some(DomainGroup::new(config.domain.as_str())),
            api_key: config.api_key.clone(),
        };
    };
    if !group_names.contains(group) {
        error!(
            "not serving allowed domain '{}': its domain_group '{group}' has to be an allowed domain that is in no other domain's group",
            config.domain
        );
        return Domain {
            group: None,
            api_key: None,
        };
    }
    Domain {
        group: Some(DomainGroup::new(group)),
        api_key: None,
    }
}

/// Domains that have no Breez API key of their own and are in no other domain's
/// group.
fn domains_without_api_key(configs: &[DomainConfig]) -> Vec<String> {
    configs
        .iter()
        .filter(|d| d.api_key.is_none() && joined_group(d).is_none())
        .map(|d| d.domain.clone())
        .collect()
}

/// A domain as the log lines name it: with its group when that is not its own.
fn describe(name: &str, domain: &Domain) -> String {
    match &domain.group {
        Some(group) if group.as_str() == name => name.to_string(),
        Some(group) => format!("{name} (group {group})"),
        None => format!("{name} (not served)"),
    }
}

fn log_loaded(map: &DomainMap) {
    let mut sorted: Vec<String> = map
        .iter()
        .map(|(name, domain)| describe(name, domain))
        .collect();
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

/// Log added/removed domains, group changes and api-key rotations so each is
/// observable.
fn log_changes(current: &DomainMap, latest: &DomainMap) {
    for (name, domain) in latest {
        let Some(old) = current.get(name) else {
            info!("allowed domain added: {}", describe(name, domain));
            continue;
        };
        if old.group != domain.group {
            info!("domain group changed: {}", describe(name, domain));
        }
        if old.api_key != domain.api_key {
            info!("Breez API key changed for domain: {}", name);
        }
    }
    for name in current.keys() {
        if !latest.contains_key(name) {
            info!("allowed domain removed: {}", name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(domain: &str, api_key: Option<&str>) -> DomainConfig {
        DomainConfig {
            domain: domain.to_string(),
            domain_group: None,
            api_key: api_key.map(str::to_string),
            jwt: None,
        }
    }

    /// `domain` configured to be in the group of `group`.
    fn joined(domain: &str, group: &str) -> DomainConfig {
        DomainConfig {
            domain_group: Some(group.to_string()),
            ..cfg(domain, None)
        }
    }

    #[test]
    fn to_map_keeps_own_key() {
        let configs = vec![cfg("a.com", Some("own")), cfg("b.com", None)];
        let map = to_map(configs);
        assert_eq!(map["a.com"].api_key.as_deref(), Some("own"));
        // A domain with no api key maps to None; the default is applied when
        // selecting the wallet, not here.
        assert_eq!(map["b.com"].api_key, None);
    }

    #[test]
    fn a_domain_is_in_the_group_it_joined() {
        let map = to_map(vec![cfg("a.com", None), joined("b.com", "a.com")]);
        assert_eq!(
            group_of(&map, "b.com", true),
            Some(DomainGroup::new("a.com"))
        );
        assert_eq!(
            group_of(&map, "a.com", true),
            Some(DomainGroup::new("a.com"))
        );
    }

    #[test]
    fn a_domain_naming_itself_is_its_own_group() {
        let map = to_map(vec![DomainConfig {
            api_key: Some("own".to_string()),
            ..joined("a.com", "a.com")
        }]);
        assert_eq!(
            group_of(&map, "a.com", true),
            Some(DomainGroup::new("a.com"))
        );
        assert_eq!(map["a.com"].api_key.as_deref(), Some("own"));
    }

    #[test]
    fn a_group_is_named_after_a_domain_in_no_other_group() {
        let map = to_map(vec![
            cfg("a.com", None),
            joined("b.com", "a.com"),
            joined("c.com", "b.com"),
        ]);
        assert_eq!(group_of(&map, "c.com", true), None);
        assert_eq!(
            group_of(&map, "b.com", true),
            Some(DomainGroup::new("a.com"))
        );

        let map = to_map(vec![joined("b.com", "missing.com")]);
        assert_eq!(group_of(&map, "b.com", true), None);
    }

    #[test]
    fn a_list_of_refused_domains_does_not_fall_open() {
        // Each is in the other's group, so neither names one.
        let map = to_map(vec![joined("a.com", "b.com"), joined("b.com", "a.com")]);
        assert_eq!(group_of(&map, "a.com", false), None);
        assert_eq!(group_of(&map, "c.com", false), None);
    }

    #[test]
    fn a_domain_in_another_group_has_no_api_key_of_its_own() {
        let map = to_map(vec![
            cfg("a.com", Some("key-a")),
            DomainConfig {
                api_key: Some("key-b".to_string()),
                ..joined("b.com", "a.com")
            },
        ]);
        assert_eq!(map["a.com"].api_key.as_deref(), Some("key-a"));
        assert_eq!(map["b.com"].api_key, None);
    }

    #[test]
    fn only_allowed_domains_are_served() {
        let map = to_map(vec![cfg("a.com", None)]);
        assert_eq!(
            group_of(&map, "a.com", true),
            Some(DomainGroup::new("a.com"))
        );
        assert_eq!(group_of(&map, "b.com", true), None);
        assert_eq!(
            group_of(&map, "b.com", false),
            None,
            "a domain missing from a list that has entries is refused on every network"
        );
    }

    #[test]
    fn an_empty_allow_list_serves_any_domain_off_mainnet_only() {
        let map = DomainMap::new();
        assert_eq!(
            group_of(&map, "a.com", false),
            Some(DomainGroup::new("a.com"))
        );
        assert_eq!(group_of(&map, "a.com", true), None);
    }

    #[test]
    fn lists_domains_without_an_api_key() {
        // Independent of any default: the warning tracks own keys.
        let configs = vec![
            cfg("a.com", Some("own")),
            cfg("b.com", None),
            joined("c.com", "a.com"),
        ];
        assert_eq!(domains_without_api_key(&configs), vec!["b.com".to_string()]);
    }
}
