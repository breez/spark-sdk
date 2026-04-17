use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tracing::{debug, error, info};

use super::repository::{WebhookConfig, WebhookRepository};

const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// In-memory cache of webhook configurations keyed by domain.
pub type WebhookConfigCache = Arc<RwLock<HashMap<String, WebhookConfig>>>;

/// Load webhook configurations from the database and start a background task
/// that periodically refreshes them.
pub async fn start<DB>(db: DB) -> Result<WebhookConfigCache, anyhow::Error>
where
    DB: WebhookRepository + Clone + Send + Sync + 'static,
{
    let initial = load_configs(&db).await?;
    let mut domains: Vec<&str> = initial.keys().map(String::as_str).collect();
    domains.sort_unstable();
    info!(
        "loaded webhook configs for {} domain(s): {}",
        domains.len(),
        domains.join(", ")
    );
    let cache = Arc::new(RwLock::new(initial));

    let cache_clone = Arc::clone(&cache);
    tokio::spawn(async move {
        debug!("Webhook config refresher started");
        loop {
            tokio::time::sleep(REFRESH_INTERVAL).await;
            refresh_once(&db, &cache_clone).await;
        }
    });

    Ok(cache)
}

async fn load_configs<DB>(db: &DB) -> Result<HashMap<String, WebhookConfig>, anyhow::Error>
where
    DB: WebhookRepository,
{
    let configs = db.list_webhook_configs().await?;
    Ok(configs.into_iter().map(|c| (c.domain.clone(), c)).collect())
}

async fn refresh_once<DB>(db: &DB, cache: &RwLock<HashMap<String, WebhookConfig>>)
where
    DB: WebhookRepository,
{
    let latest = match load_configs(db).await {
        Ok(m) => m,
        Err(e) => {
            error!("Failed to refresh webhook configs: {e}");
            return;
        }
    };

    {
        let current = cache.read().await;
        if configs_equal(&current, &latest) {
            return;
        }
        log_changes(&current, &latest);
    }

    *cache.write().await = latest;
}

fn configs_equal(a: &HashMap<String, WebhookConfig>, b: &HashMap<String, WebhookConfig>) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().all(|(k, v)| {
        b.get(k)
            .is_some_and(|bv| bv.url == v.url && bv.secret == v.secret)
    })
}

fn log_changes(current: &HashMap<String, WebhookConfig>, latest: &HashMap<String, WebhookConfig>) {
    for domain in latest.keys() {
        if !current.contains_key(domain) {
            info!("webhook config added: {domain}");
        }
    }
    for domain in current.keys() {
        if !latest.contains_key(domain.as_str()) {
            info!("webhook config removed: {domain}");
        }
    }
    for (domain, new) in latest {
        if let Some(old) = current.get(domain) {
            if old.url != new.url {
                info!("webhook config updated: {domain}: url changed");
            }
            if old.secret != new.secret {
                info!("webhook config updated: {domain}: secret changed");
            }
        }
    }
}
