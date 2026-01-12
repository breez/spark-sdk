mod config;
mod fiat_currencies;
mod getting_started;
mod htlcs;
mod issuing_tokens;
mod lightning_address;
mod list_payments;
mod lnurl_pay;
mod messages;
mod parsing_inputs;
mod receive_payment;
mod refunding_payments;
mod sdk_building;
mod send_payment;
mod tokens;
mod user_settings;
mod optimize;
mod external_signer;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    getting_started::getting_started_logging("./.data".to_string())?;

    let sdk = getting_started::init_sdk().await?;
    let listener_id =
        getting_started::add_event_listener(&sdk, Box::new(getting_started::SdkEventListener {}))
            .await?;
    getting_started::getting_started_node_info(&sdk).await?;
    getting_started::remove_event_listener(&sdk, &listener_id).await?;
    getting_started::disconnect(&sdk).await?;

    Ok(())
}
