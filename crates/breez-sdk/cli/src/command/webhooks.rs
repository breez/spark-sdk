use breez_sdk_spark::{BreezSdk, DeleteWebhookRequest, RegisterWebhookRequest, WebhookEventType};
use clap::Subcommand;

use crate::command::print_value;

#[derive(Clone, Debug, Subcommand)]
pub enum WebhookCommand {
    /// Register a new webhook
    Register {
        /// The URL to receive webhook notifications
        url: String,
        /// Event types to subscribe to (e.g. lightning-receive-finished, lightning-send-finished, coop-exit-finished, static-deposit-finished)
        #[arg(short, long, required = true)]
        event_types: Vec<WebhookEventType>,
    },
    /// Delete a webhook
    Delete {
        /// The ID of the webhook to delete
        webhook_id: String,
    },
    /// List all registered webhooks
    List,
}

pub async fn handle_command(
    sdk: &BreezSdk,
    command: WebhookCommand,
) -> Result<bool, anyhow::Error> {
    match command {
        WebhookCommand::Register { url, event_types } => {
            let response = sdk
                .register_webhook(RegisterWebhookRequest { url, event_types })
                .await?;
            print_value(&response)?;
            Ok(true)
        }
        WebhookCommand::Delete { webhook_id } => {
            let response = sdk
                .delete_webhook(DeleteWebhookRequest { webhook_id })
                .await?;
            print_value(&response)?;
            Ok(true)
        }
        WebhookCommand::List => {
            let response = sdk.list_webhooks().await?;
            print_value(&response)?;
            Ok(true)
        }
    }
}
