use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, Semaphore};
use tracing::{debug, error, warn};

use super::repository::{WebhookDelivery, WebhookRepository};
use crate::time::now_millis;

/// Retry configuration.
const BASE_RETRY_DELAY_MS: i64 = 30_000; // 30 seconds
const RETRY_MULTIPLIER: f64 = 1.5;

/// Maximum number of concurrent in-flight webhook deliveries per URL.
const MAX_CONCURRENT_PER_URL: usize = 20;

/// Maximum length of error response body to store.
const MAX_ERROR_BODY_LEN: usize = 512;

/// Webhook request timeout in seconds.
const WEBHOOK_TIMEOUT_SECS: u64 = 30;

/// How often to run the webhook delivery cleanup (1 hour).
#[allow(unknown_lints, clippy::duration_suboptimal_units)]
const CLEANUP_INTERVAL: tokio::time::Duration = tokio::time::Duration::from_secs(60 * 60);

/// Per-URL concurrency limiter for webhook delivery.
pub(crate) type UrlSemaphores = Arc<Mutex<HashMap<String, Arc<Semaphore>>>>;

/// Start all webhook-related background processors.
pub fn start_background_processor<DB>(
    db: DB,
    http_client: bitreq::Client,
    trigger_rx: tokio::sync::watch::Receiver<()>,
    webhook_delivery_ttl_days: u32,
) where
    DB: WebhookRepository + Clone + Send + Sync + 'static,
{
    tokio::spawn(webhook_delivery_processor(
        db.clone(),
        http_client,
        trigger_rx,
    ));
    tokio::spawn(webhook_cleanup_processor(db, webhook_delivery_ttl_days));
}

/// Calculate the next retry delay with exponential backoff.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
pub(crate) fn next_retry_delay(retry_count: i32) -> i64 {
    (BASE_RETRY_DELAY_MS as f64 * RETRY_MULTIPLIER.powi(retry_count)) as i64
}

/// Start the webhook delivery processor.
async fn webhook_delivery_processor<DB>(
    db: DB,
    http_client: bitreq::Client,
    mut trigger_rx: tokio::sync::watch::Receiver<()>,
) where
    DB: WebhookRepository + Clone + Send + Sync + 'static,
{
    debug!("Webhook delivery processor started");

    let url_semaphores: UrlSemaphores = Arc::new(Mutex::new(HashMap::new()));

    // Process any pending items on startup
    process_pending_webhook_deliveries(&db, &http_client, &url_semaphores).await;

    // Wait for triggers
    loop {
        tokio::select! {
            result = trigger_rx.changed() => {
                if result.is_err() {
                    debug!("Webhook delivery processor trigger channel closed, exiting");
                    return;
                }
            }
            () = tokio::time::sleep(tokio::time::Duration::from_mins(1)) => {}
        }

        process_pending_webhook_deliveries(&db, &http_client, &url_semaphores).await;
    }
}

/// Start the webhook cleanup processor.
async fn webhook_cleanup_processor<DB>(db: DB, webhook_delivery_ttl_days: u32)
where
    DB: WebhookRepository + Clone + Send + Sync + 'static,
{
    let ttl_ms = i64::from(webhook_delivery_ttl_days).saturating_mul(24 * 60 * 60 * 1000);
    let mut cleanup_interval = tokio::time::interval(CLEANUP_INTERVAL);

    loop {
        cleanup_interval.tick().await;
        cleanup_old_webhook_deliveries(&db, ttl_ms).await;
    }
}

/// Delete webhook deliveries older than the configured TTL. Applies to both
/// succeeded and failed deliveries, serving as the final retention window for
/// audit/debugging purposes.
async fn cleanup_old_webhook_deliveries<DB>(db: &DB, ttl_ms: i64)
where
    DB: WebhookRepository + Clone + Send + Sync + 'static,
{
    let cutoff = now_millis().saturating_sub(ttl_ms);
    match db.delete_webhook_deliveries_older_than(cutoff).await {
        Ok(0) => {}
        Ok(count) => debug!("Cleaned up {count} old webhook deliveries"),
        Err(e) => error!("Failed to clean up old webhook deliveries: {e}"),
    }
}

/// Get or create the semaphore for a given URL.
async fn get_semaphore(semaphores: &UrlSemaphores, url: &str) -> Arc<Semaphore> {
    let mut map = semaphores.lock().await;
    Arc::clone(
        map.entry(url.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(MAX_CONCURRENT_PER_URL))),
    )
}

/// Claim and deliver pending webhooks. The query returns at most one
/// delivery per URL, so one slow domain cannot starve others.
pub(crate) async fn process_pending_webhook_deliveries<DB>(
    db: &DB,
    http_client: &bitreq::Client,
    url_semaphores: &UrlSemaphores,
) where
    DB: WebhookRepository + Clone + Send + Sync + 'static,
{
    let deliveries = match db.take_pending_webhook_deliveries().await {
        Ok(d) => d,
        Err(e) => {
            error!("Failed to claim webhook deliveries: {}", e);
            return;
        }
    };

    if deliveries.is_empty() {
        return;
    }

    debug!("Claimed {} webhook deliveries", deliveries.len());

    let mut unclaim_ids = Vec::new();

    for delivery in deliveries {
        let sem = get_semaphore(url_semaphores, &delivery.url).await;

        if let Ok(permit) = sem.clone().try_acquire_owned() {
            let db = db.clone();
            let client = http_client.clone();
            tokio::spawn(async move {
                process_webhook_delivery(&db, &client, &delivery).await;
                drop(permit);
            });
        } else {
            unclaim_ids.push(delivery.id);
        }
    }

    if !unclaim_ids.is_empty() {
        debug!("Unclaiming {} webhook deliveries", unclaim_ids.len());
        if let Err(e) = db.unclaim_webhook_deliveries(&unclaim_ids).await {
            error!("Failed to unclaim webhook deliveries: {}", e);
        }
    }
}

/// Process a single webhook delivery attempt.
async fn process_webhook_delivery<DB>(
    db: &DB,
    http_client: &bitreq::Client,
    delivery: &WebhookDelivery,
) where
    DB: WebhookRepository + Clone + Send + Sync + 'static,
{
    let now = now_millis();

    match send_webhook(http_client, &delivery.url, &delivery.payload).await {
        Ok(()) => {
            debug!("Webhook delivery {} succeeded", delivery.id);
            if let Err(e) = db.update_webhook_delivery_success(delivery.id, now).await {
                error!(
                    "Failed to update webhook delivery success {}: {}",
                    delivery.id, e
                );
            }
        }
        Err(WebhookError { status_code, body }) => {
            warn!(
                "Webhook delivery {} failed: status={:?}",
                delivery.id, status_code
            );

            let retry_count = delivery.retry_count.saturating_add(1);
            let next_retry_at = now.saturating_add(next_retry_delay(retry_count));

            if let Err(e) = db
                .update_webhook_delivery_failure(
                    delivery.id,
                    retry_count,
                    next_retry_at,
                    status_code,
                    body.as_deref(),
                )
                .await
            {
                error!(
                    "Failed to update webhook delivery failure {}: {}",
                    delivery.id, e
                );
            }
        }
    }
}

struct WebhookError {
    status_code: Option<i32>,
    body: Option<String>,
}

async fn send_webhook(
    http_client: &bitreq::Client,
    url: &str,
    payload_json: &str,
) -> Result<(), WebhookError> {
    let req = bitreq::post(url)
        .with_header("Content-Type", "application/json")
        .with_body(payload_json)
        .with_timeout(WEBHOOK_TIMEOUT_SECS);

    let response = http_client
        .send_async(req)
        .await
        .map_err(|e| WebhookError {
            status_code: None,
            body: Some(truncate_string(&format!("{e:?}"), MAX_ERROR_BODY_LEN)),
        })?;

    if (200..300).contains(&response.status_code) {
        return Ok(());
    }

    let body = response
        .as_str()
        .ok()
        .map(|b| truncate_string(b, MAX_ERROR_BODY_LEN));

    Err(WebhookError {
        status_code: Some(response.status_code),
        body,
    })
}

fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    let end = s.floor_char_boundary(max_len);
    format!("{}...", &s[..end])
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use axum::Router;
    use axum::routing::post;
    use sqlx::{Row, SqlitePool};
    use tokio::sync::Semaphore;

    use super::*;
    use crate::webhooks::WebhookRepository;
    use crate::webhooks::repository::NewWebhookDelivery;

    // ── Test helpers ────────────────────────────────────────────────────

    async fn setup_test_db() -> (crate::sqlite::LnurlRepository, SqlitePool) {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect(":memory:")
            .await
            .unwrap();
        crate::sqlite::run_migrations(&pool).await.unwrap();
        let db = crate::sqlite::LnurlRepository::new(pool.clone());
        (db, pool)
    }

    async fn insert_delivery(db: &impl WebhookRepository, identifier: &str, url: &str) {
        let delivery = NewWebhookDelivery {
            identifier: identifier.to_string(),
            url: url.to_string(),
            payload: r#"{"event":"invoice.paid","id":"test123"}"#.to_string(),
        };
        db.insert_webhook_deliveries(&[delivery]).await.unwrap();
    }

    async fn get_delivery_by_identifier(
        pool: &SqlitePool,
        identifier: &str,
    ) -> sqlx::sqlite::SqliteRow {
        sqlx::query(
            "SELECT id, identifier, url, payload, created_at, succeeded_at,
                    retry_count, next_retry_at, claimed_at,
                    last_error_status_code, last_error_body
             FROM webhook_deliveries WHERE identifier = $1",
        )
        .bind(identifier)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    fn new_semaphores() -> UrlSemaphores {
        Arc::new(tokio::sync::Mutex::new(HashMap::new()))
    }

    /// Start a mock HTTP server. Returns the base URL (e.g. `http://127.0.0.1:12345`).
    async fn start_mock_server(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://127.0.0.1:{}", addr.port())
    }

    // ── Tests ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn successful_delivery_marks_succeeded() {
        let (db, pool) = setup_test_db().await;

        let router = Router::new().route("/hook", post(|| async { axum::http::StatusCode::OK }));
        let base_url = start_mock_server(router).await;
        let url = format!("{base_url}/hook");

        insert_delivery(&db, "success_1", &url).await;

        let client = bitreq::Client::new(10);
        let semaphores = new_semaphores();

        process_pending_webhook_deliveries(&db, &client, &semaphores).await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        let row = get_delivery_by_identifier(&pool, "success_1").await;
        let succeeded_at: Option<i64> = row.try_get("succeeded_at").unwrap();
        let retry_count: i32 = row.try_get("retry_count").unwrap();

        assert!(succeeded_at.is_some(), "succeeded_at should be set");
        assert_eq!(retry_count, 0, "retry_count should be 0");
    }

    #[tokio::test]
    async fn server_error_causes_retry() {
        let (db, pool) = setup_test_db().await;

        let router = Router::new().route(
            "/hook",
            post(|| async {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal Server Error",
                )
            }),
        );
        let base_url = start_mock_server(router).await;
        let url = format!("{base_url}/hook");

        insert_delivery(&db, "error_1", &url).await;

        let client = bitreq::Client::new(10);
        let semaphores = new_semaphores();

        process_pending_webhook_deliveries(&db, &client, &semaphores).await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        let row = get_delivery_by_identifier(&pool, "error_1").await;
        let succeeded_at: Option<i64> = row.try_get("succeeded_at").unwrap();
        let retry_count: i32 = row.try_get("retry_count").unwrap();
        let next_retry_at: i64 = row.try_get("next_retry_at").unwrap();
        let last_error_status_code: Option<i32> = row.try_get("last_error_status_code").unwrap();
        let last_error_body: Option<String> = row.try_get("last_error_body").unwrap();

        assert!(succeeded_at.is_none(), "succeeded_at should be NULL");
        assert_eq!(retry_count, 1, "retry_count should be 1");
        assert!(
            next_retry_at > now_millis(),
            "next_retry_at should be in the future"
        );
        assert_eq!(last_error_status_code, Some(500));
        assert_eq!(last_error_body.as_deref(), Some("Internal Server Error"));
    }

    #[tokio::test]
    async fn connection_error_causes_retry() {
        let (db, pool) = setup_test_db().await;

        // Port 1 — nothing is listening there.
        let url = "http://127.0.0.1:1/hook";
        insert_delivery(&db, "conn_err_1", url).await;

        let client = bitreq::Client::new(10);
        let semaphores = new_semaphores();

        process_pending_webhook_deliveries(&db, &client, &semaphores).await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        let row = get_delivery_by_identifier(&pool, "conn_err_1").await;
        let succeeded_at: Option<i64> = row.try_get("succeeded_at").unwrap();
        let retry_count: i32 = row.try_get("retry_count").unwrap();
        let last_error_status_code: Option<i32> = row.try_get("last_error_status_code").unwrap();
        let last_error_body: Option<String> = row.try_get("last_error_body").unwrap();

        assert!(succeeded_at.is_none(), "succeeded_at should be NULL");
        assert_eq!(retry_count, 1, "retry_count should be 1");
        assert!(
            last_error_status_code.is_none(),
            "no HTTP response means no status code"
        );
        assert!(
            last_error_body.is_some(),
            "error body should contain the connection error description"
        );
    }

    #[tokio::test]
    async fn failed_delivery_is_retried_and_succeeds() {
        let (db, pool) = setup_test_db().await;

        let counter = Arc::new(AtomicUsize::new(0));

        let router = Router::new().route(
            "/hook",
            post({
                let counter = Arc::clone(&counter);
                move || {
                    let counter = Arc::clone(&counter);
                    async move {
                        let n = counter.fetch_add(1, Ordering::SeqCst);
                        if n == 0 {
                            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "oops")
                        } else {
                            (axum::http::StatusCode::OK, "ok")
                        }
                    }
                }
            }),
        );
        let base_url = start_mock_server(router).await;
        let url = format!("{base_url}/hook");

        insert_delivery(&db, "retry_1", &url).await;

        let client = bitreq::Client::new(10);
        let semaphores = new_semaphores();

        // First attempt — should fail.
        process_pending_webhook_deliveries(&db, &client, &semaphores).await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        let row = get_delivery_by_identifier(&pool, "retry_1").await;
        let retry_count: i32 = row.try_get("retry_count").unwrap();
        let succeeded_at: Option<i64> = row.try_get("succeeded_at").unwrap();
        assert_eq!(retry_count, 1);
        assert!(succeeded_at.is_none());

        // Manually make the delivery eligible for retry by setting next_retry_at to now.
        let id: i64 = row.try_get("id").unwrap();
        sqlx::query(
            "UPDATE webhook_deliveries SET next_retry_at = $1, claimed_at = NULL WHERE id = $2",
        )
        .bind(now_millis())
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();

        // Second attempt — should succeed.
        process_pending_webhook_deliveries(&db, &client, &semaphores).await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        let row = get_delivery_by_identifier(&pool, "retry_1").await;
        let succeeded_at: Option<i64> = row.try_get("succeeded_at").unwrap();
        assert!(
            succeeded_at.is_some(),
            "delivery should have succeeded on retry"
        );
    }

    #[tokio::test]
    async fn error_body_is_truncated() {
        let (db, pool) = setup_test_db().await;

        let long_body: String = "x".repeat(1000);
        let router = Router::new().route(
            "/hook",
            post({
                let long_body = long_body.clone();
                move || {
                    let body = long_body.clone();
                    async move { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, body) }
                }
            }),
        );
        let base_url = start_mock_server(router).await;
        let url = format!("{base_url}/hook");

        insert_delivery(&db, "truncate_1", &url).await;

        let client = bitreq::Client::new(10);
        let semaphores = new_semaphores();

        process_pending_webhook_deliveries(&db, &client, &semaphores).await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        let row = get_delivery_by_identifier(&pool, "truncate_1").await;
        let last_error_body: Option<String> = row.try_get("last_error_body").unwrap();
        let body = last_error_body.expect("error body should be present");

        // 512 chars + "..." = 515 max
        assert!(
            body.len() <= 515,
            "error body should be at most 515 chars, got {}",
            body.len()
        );
        assert!(
            body.ends_with("..."),
            "truncated body should end with '...'"
        );
    }

    #[tokio::test]
    async fn exponential_backoff_delays() {
        assert_eq!(next_retry_delay(0), 30_000);
        assert_eq!(next_retry_delay(1), 45_000);

        // retry 5: 30_000 * 1.5^5 = 30_000 * 7.59375 = 227_812.5 → 227_812
        let delay5 = next_retry_delay(5);
        assert!(
            (227_812..=227_813).contains(&delay5),
            "retry 5 delay should be ~227_812, got {delay5}"
        );

        // Very high retry counts keep growing (no cap).
        assert!(next_retry_delay(100) > next_retry_delay(5));
    }

    #[tokio::test]
    async fn slow_server_does_not_block_fast_server() {
        let (db, pool) = setup_test_db().await;

        // Slow server: sleeps 2 seconds before responding.
        let slow_router = Router::new().route(
            "/hook",
            post(|| async {
                tokio::time::sleep(Duration::from_secs(2)).await;
                axum::http::StatusCode::OK
            }),
        );
        let slow_url = format!("{}/hook", start_mock_server(slow_router).await);

        // Fast server: responds immediately.
        let fast_router =
            Router::new().route("/hook", post(|| async { axum::http::StatusCode::OK }));
        let fast_url = format!("{}/hook", start_mock_server(fast_router).await);

        insert_delivery(&db, "slow_1", &slow_url).await;
        insert_delivery(&db, "fast_1", &fast_url).await;

        let client = bitreq::Client::new(10);
        let semaphores = new_semaphores();

        let result = tokio::time::timeout(Duration::from_secs(4), async {
            process_pending_webhook_deliveries(&db, &client, &semaphores).await;
            // Wait enough for the slow server to finish.
            tokio::time::sleep(Duration::from_millis(2500)).await;
        })
        .await;
        assert!(result.is_ok(), "both deliveries should complete within 4s");

        let slow_row = get_delivery_by_identifier(&pool, "slow_1").await;
        let fast_row = get_delivery_by_identifier(&pool, "fast_1").await;

        let slow_succeeded: Option<i64> = slow_row.try_get("succeeded_at").unwrap();
        let fast_succeeded: Option<i64> = fast_row.try_get("succeeded_at").unwrap();

        assert!(
            slow_succeeded.is_some(),
            "slow delivery should have succeeded"
        );
        assert!(
            fast_succeeded.is_some(),
            "fast delivery should have succeeded"
        );
    }

    #[tokio::test]
    async fn per_url_throttling_unclaims_excess() {
        let (db, pool) = setup_test_db().await;

        // Use a real server so the URL is valid, but we expect the delivery never
        // to be attempted because the semaphore has no available permits.
        let router = Router::new().route("/hook", post(|| async { axum::http::StatusCode::OK }));
        let base_url = start_mock_server(router).await;
        let url = format!("{base_url}/hook");

        insert_delivery(&db, "throttle_1", &url).await;

        let client = bitreq::Client::new(10);

        // Pre-fill semaphores so this URL has 0 available permits.
        let semaphores = new_semaphores();
        {
            let mut map = semaphores.lock().await;
            // Create a semaphore with 0 permits — all slots "in use".
            map.insert(url.clone(), Arc::new(Semaphore::new(0)));
        }

        process_pending_webhook_deliveries(&db, &client, &semaphores).await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        let row = get_delivery_by_identifier(&pool, "throttle_1").await;
        let claimed_at: Option<i64> = row.try_get("claimed_at").unwrap();
        let retry_count: i32 = row.try_get("retry_count").unwrap();
        let succeeded_at: Option<i64> = row.try_get("succeeded_at").unwrap();

        assert!(claimed_at.is_none(), "delivery should have been unclaimed");
        assert_eq!(retry_count, 0, "retry_count should remain 0");
        assert!(succeeded_at.is_none(), "delivery should not have succeeded");
    }
}
