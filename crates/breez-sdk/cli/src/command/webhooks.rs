use breez_sdk_spark::{
    BreezSdk, RegisterWebhookRequest, UnregisterWebhookRequest, WebhookEventType,
};
use clap::Subcommand;

use crate::command::print_value;

#[derive(Clone, Debug, Subcommand)]
pub enum WebhookCommand {
    /// Register a new webhook
    Register {
        /// URL that will receive webhook notifications
        url: String,
        /// Secret for HMAC-SHA256 signature verification
        secret: String,
        /// Event types to subscribe to (lightning-receive, lightning-send, coop-exit, static-deposit)
        #[arg(required = true, num_args = 1..)]
        events: Vec<String>,
    },
    /// Unregister a webhook
    Unregister {
        /// ID of the webhook to unregister
        webhook_id: String,
    },
    /// List all registered webhooks
    List,
}

fn parse_event_type(s: &str) -> Result<WebhookEventType, anyhow::Error> {
    match s {
        "lightning-receive" => Ok(WebhookEventType::LightningReceiveFinished),
        "lightning-send" => Ok(WebhookEventType::LightningSendFinished),
        "coop-exit" => Ok(WebhookEventType::CoopExitFinished),
        "static-deposit" => Ok(WebhookEventType::StaticDepositFinished),
        _ => Err(anyhow::anyhow!(
            "Unknown event type: {s}. Valid values: lightning-receive, lightning-send, coop-exit, static-deposit"
        )),
    }
}

pub async fn handle_command(
    sdk: &BreezSdk,
    command: WebhookCommand,
) -> Result<bool, anyhow::Error> {
    match command {
        WebhookCommand::Register {
            url,
            secret,
            events,
        } => {
            let event_types = events
                .iter()
                .map(|e| parse_event_type(e))
                .collect::<Result<Vec<_>, _>>()?;
            let response = sdk
                .register_webhook(RegisterWebhookRequest {
                    url,
                    secret,
                    event_types,
                })
                .await?;
            print_value(&response)?;
            Ok(true)
        }
        WebhookCommand::Unregister { webhook_id } => {
            sdk.unregister_webhook(UnregisterWebhookRequest { webhook_id })
                .await?;
            println!("Webhook unregistered successfully");
            Ok(true)
        }
        WebhookCommand::List => {
            let webhooks = sdk.list_webhooks().await?;
            print_value(&webhooks)?;
            Ok(true)
        }
    }
}
