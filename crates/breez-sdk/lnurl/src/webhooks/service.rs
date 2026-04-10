use super::repository::{NewWebhookDelivery, WebhookRepository, WebhookRepositoryError};

pub struct WebhookService<DB> {
    db: DB,
}

impl<DB: WebhookRepository> WebhookService<DB> {
    pub fn new(db: DB) -> Self {
        Self { db }
    }

    /// Enqueue webhook deliveries for background processing.
    pub async fn enqueue(
        &self,
        deliveries: &[NewWebhookDelivery],
    ) -> Result<(), WebhookRepositoryError> {
        if deliveries.is_empty() {
            return Ok(());
        }

        self.db.insert_webhook_deliveries(deliveries).await
    }
}

impl<DB: Clone> Clone for WebhookService<DB> {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
        }
    }
}
