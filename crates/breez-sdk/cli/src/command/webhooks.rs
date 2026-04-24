use breez_sdk_spark::{
    BreezSdk, RegisterWebhookRequest, UnregisterWebhookRequest, WebhookEventType,
};
use clap::{Subcommand, ValueEnum};

use crate::command::print_value;

#[derive(Clone, Copy, Debug, ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum WebhookEventTypeArg {
    LightningReceive,
    LightningSend,
    CoopExit,
    StaticDeposit,
}

impl From<WebhookEventTypeArg> for WebhookEventType {
    fn from(value: WebhookEventTypeArg) -> Self {
        match value {
            WebhookEventTypeArg::LightningReceive => WebhookEventType::LightningReceiveFinished,
            WebhookEventTypeArg::LightningSend => WebhookEventType::LightningSendFinished,
            WebhookEventTypeArg::CoopExit => WebhookEventType::CoopExitFinished,
            WebhookEventTypeArg::StaticDeposit => WebhookEventType::StaticDepositFinished,
        }
    }
}

#[derive(Clone, Debug, Subcommand)]
pub enum WebhookCommand {
    /// Register a new webhook
    Register {
        /// URL that will receive webhook notifications
        url: String,
        /// Secret for HMAC-SHA256 signature verification
        secret: String,
        /// Event types to subscribe to
        #[arg(required = true, num_args = 1.., value_enum)]
        events: Vec<WebhookEventTypeArg>,
    },
    /// Unregister a webhook
    Unregister {
        /// ID of the webhook to unregister
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
        WebhookCommand::Register {
            url,
            secret,
            events,
        } => {
            let event_types = events.into_iter().map(Into::into).collect();
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
