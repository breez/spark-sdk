mod buying_bitcoin;
mod config;
mod external_signer;
mod fiat_currencies;
mod getting_started;
mod htlcs;
mod issuing_tokens;
mod lightning_address;
mod list_payments;
mod lnurl_auth;
mod lnurl_pay;
mod lnurl_withdraw;
mod messages;
mod optimize;
mod parsing_inputs;
mod receive_payment;
mod refunding_payments;
mod sdk_building;
mod send_payment;
mod tokens;
mod user_settings;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    getting_started::getting_started_logging("./.data".to_string())?;

    let client = getting_started::init_sdk().await?;
    let listener_id =
        getting_started::add_event_listener(&client, Box::new(getting_started::SdkEventListener {}))
            .await?;
    getting_started::getting_started_node_info(&client).await?;
    getting_started::remove_event_listener(&client, &listener_id).await?;
    getting_started::disconnect(&client).await?;

    Ok(())
}
