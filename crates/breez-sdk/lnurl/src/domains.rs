use std::{collections::HashSet, sync::Arc, time::Duration};

use tokio::sync::RwLock;
use tracing::{debug, error, info};

use crate::repository::{LnurlRepository, LnurlRepositoryError};

const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Load the allowed domains from the database and start a background task
/// that periodically refreshes them. Returns the shared handle that should
/// be stored on `State` so request handlers observe updates.
pub async fn start<DB>(db: DB) -> Result<Arc<RwLock<HashSet<String>>>, LnurlRepositoryError>
where
    DB: LnurlRepository + Clone + Send + Sync + 'static,
{
    let initial: HashSet<String> = db.list_domains().await?.into_iter().collect();
    let mut sorted: Vec<&str> = initial.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    info!("loaded allowed domains: {}", sorted.join(", "));
    let domains = Arc::new(RwLock::new(initial));

    let db_clone = db.clone();
    let domains_clone = Arc::clone(&domains);
    tokio::spawn(async move {
        debug!("Allowed domains refresher started");
        loop {
            tokio::time::sleep(REFRESH_INTERVAL).await;
            refresh_once(&db_clone, &domains_clone).await;
        }
    });

    Ok(domains)
}

async fn refresh_once<DB>(db: &DB, domains: &RwLock<HashSet<String>>)
where
    DB: LnurlRepository,
{
    let latest: HashSet<String> = match db.list_domains().await {
        Ok(list) => list.into_iter().collect(),
        Err(e) => {
            error!("Failed to refresh allowed domains: {}", e);
            return;
        }
    };

    let (added, removed): (Vec<String>, Vec<String>) = {
        let current = domains.read().await;
        if *current == latest {
            return;
        }
        (
            latest.difference(&current).cloned().collect(),
            current.difference(&latest).cloned().collect(),
        )
    };

    for domain in &added {
        info!("allowed domain added: {}", domain);
    }
    for domain in &removed {
        info!("allowed domain removed: {}", domain);
    }

    *domains.write().await = latest;
}
