#[derive(Debug, thiserror::Error)]
pub enum WebhookRepositoryError {
    #[error("database error: {0}")]
    General(anyhow::Error),
}

impl From<sqlx::Error> for WebhookRepositoryError {
    fn from(e: sqlx::Error) -> Self {
        Self::General(e.into())
    }
}

#[derive(Debug, Clone)]
pub struct NewWebhookDelivery {
    pub identifier: String,
    pub url: String,
    pub payload: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct WebhookDelivery {
    pub id: i64,
    pub identifier: String,
    pub url: String,
    pub payload: String,
    pub created_at: i64,
    pub retry_count: i32,
    pub next_retry_at: i64,
}

#[async_trait::async_trait]
pub trait WebhookRepository {
    /// Batch insert pending webhook deliveries (idempotent via ON CONFLICT DO NOTHING).
    async fn insert_webhook_deliveries(
        &self,
        deliveries: &[NewWebhookDelivery],
    ) -> Result<(), WebhookRepositoryError>;

    /// Claim pending webhook deliveries ready for processing
    /// (`next_retry_at` <= now, not yet succeeded, not recently claimed).
    /// Returns at most one delivery per unique URL so that one slow domain
    /// cannot starve others.
    async fn take_pending_webhook_deliveries(
        &self,
    ) -> Result<Vec<WebhookDelivery>, WebhookRepositoryError>;

    /// Mark a webhook delivery as successfully delivered.
    async fn update_webhook_delivery_success(
        &self,
        id: i64,
        succeeded_at: i64,
    ) -> Result<(), WebhookRepositoryError>;

    /// Record a webhook delivery failure and schedule the next retry.
    async fn update_webhook_delivery_failure(
        &self,
        id: i64,
        retry_count: i32,
        next_retry_at: i64,
        status_code: Option<i32>,
        body: Option<&str>,
    ) -> Result<(), WebhookRepositoryError>;

    /// Release claimed webhook deliveries back to the queue so they can be
    /// picked up again in a future poll cycle.
    async fn unclaim_webhook_deliveries(&self, ids: &[i64]) -> Result<(), WebhookRepositoryError>;

    /// Delete webhook deliveries older than the given timestamp.
    async fn delete_webhook_deliveries_older_than(
        &self,
        before: i64,
    ) -> Result<u64, WebhookRepositoryError>;
}

#[cfg(test)]
pub mod shared_tests {
    use super::{NewWebhookDelivery, WebhookRepository};
    use crate::time::now_millis;

    pub async fn webhook_delivery_success_marks_succeeded<DB>(db: &DB)
    where
        DB: WebhookRepository + Clone + Send + Sync + 'static,
    {
        let now = now_millis();
        let delivery = NewWebhookDelivery {
            identifier: "success_test".to_string(),
            url: "https://success.example.com/hook".to_string(),
            payload: r#"{"test":true}"#.to_string(),
        };
        db.insert_webhook_deliveries(&[delivery]).await.unwrap();

        let claimed = db.take_pending_webhook_deliveries().await.unwrap();
        assert_eq!(claimed.len(), 1);

        db.update_webhook_delivery_success(claimed[0].id, now)
            .await
            .unwrap();

        // Succeeded delivery should no longer be claimable
        let claimed_again = db.take_pending_webhook_deliveries().await.unwrap();
        assert!(
            claimed_again.is_empty(),
            "succeeded delivery should not be claimable"
        );
    }

    pub async fn webhook_delivery_failure_schedules_retry<DB>(db: &DB)
    where
        DB: WebhookRepository + Clone + Send + Sync + 'static,
    {
        let now = now_millis();
        let delivery = NewWebhookDelivery {
            identifier: "failure_test".to_string(),
            url: "https://failure.example.com/hook".to_string(),
            payload: r#"{"test":true}"#.to_string(),
        };
        db.insert_webhook_deliveries(&[delivery]).await.unwrap();

        let claimed = db.take_pending_webhook_deliveries().await.unwrap();
        assert_eq!(claimed.len(), 1);

        let future = now.saturating_add(999_999_999);
        db.update_webhook_delivery_failure(claimed[0].id, 1, future, Some(500), Some("error"))
            .await
            .unwrap();

        // Should not be claimable yet (next_retry_at is far in the future)
        let claimed_again = db.take_pending_webhook_deliveries().await.unwrap();
        assert!(
            claimed_again.is_empty(),
            "failed delivery with future retry should not be claimable"
        );
    }

    pub async fn delete_webhook_deliveries_older_than_removes_old<DB>(db: &DB)
    where
        DB: WebhookRepository + Clone + Send + Sync + 'static,
    {
        let delivery = NewWebhookDelivery {
            identifier: "cleanup_delivery".to_string(),
            url: "https://cleanup.example.com/hook".to_string(),
            payload: r#"{"test":true}"#.to_string(),
        };
        db.insert_webhook_deliveries(&[delivery]).await.unwrap();

        // Cutoff in the past — should not delete anything
        let deleted = db.delete_webhook_deliveries_older_than(0).await.unwrap();
        assert_eq!(deleted, 0);

        // Cutoff far in the future — should delete the delivery
        let far_future = now_millis().saturating_add(999_999_999);
        let deleted = db
            .delete_webhook_deliveries_older_than(far_future)
            .await
            .unwrap();
        assert_eq!(deleted, 1);

        // Nothing left
        let remaining = db.take_pending_webhook_deliveries().await.unwrap();
        assert!(remaining.is_empty());
    }
}

#[cfg(test)]
mod sqlite_tests {
    use super::shared_tests;

    async fn setup_test_db() -> crate::sqlite::LnurlRepository {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect(":memory:")
            .await
            .unwrap();
        crate::sqlite::run_migrations(&pool).await.unwrap();
        crate::sqlite::LnurlRepository::new(pool)
    }

    #[tokio::test]
    async fn webhook_delivery_success_marks_succeeded() {
        let db = setup_test_db().await;
        shared_tests::webhook_delivery_success_marks_succeeded(&db).await;
    }

    #[tokio::test]
    async fn webhook_delivery_failure_schedules_retry() {
        let db = setup_test_db().await;
        shared_tests::webhook_delivery_failure_schedules_retry(&db).await;
    }

    #[tokio::test]
    async fn delete_webhook_deliveries_older_than_removes_old() {
        let db = setup_test_db().await;
        shared_tests::delete_webhook_deliveries_older_than_removes_old(&db).await;
    }
}

// PostgreSQL tests - only run when LNURL_TEST_POSTGRES_URL is set.
// Example: LNURL_TEST_POSTGRES_URL="postgres://user:pass@localhost/lnurl_test" cargo test
#[cfg(test)]
mod postgres_tests {
    use super::shared_tests;

    async fn setup_test_db() -> Option<crate::postgresql::LnurlRepository> {
        let url = std::env::var("LNURL_TEST_POSTGRES_URL").ok()?;
        let pool = sqlx::PgPool::connect(&url).await.ok()?;
        crate::postgresql::run_migrations(&pool).await.ok()?;

        sqlx::query("DELETE FROM webhook_deliveries")
            .execute(&pool)
            .await
            .ok()?;

        Some(crate::postgresql::LnurlRepository::new(pool))
    }

    #[tokio::test]
    async fn webhook_delivery_success_marks_succeeded() {
        let Some(db) = setup_test_db().await else {
            return;
        };
        shared_tests::webhook_delivery_success_marks_succeeded(&db).await;
    }

    #[tokio::test]
    async fn webhook_delivery_failure_schedules_retry() {
        let Some(db) = setup_test_db().await else {
            return;
        };
        shared_tests::webhook_delivery_failure_schedules_retry(&db).await;
    }

    #[tokio::test]
    async fn delete_webhook_deliveries_older_than_removes_old() {
        let Some(db) = setup_test_db().await else {
            return;
        };
        shared_tests::delete_webhook_deliveries_older_than_removes_old(&db).await;
    }
}
