mod getting_started;
mod list_payments;
mod lnurl_pay;
mod parsing_inputs;
mod receive_payment;
mod send_payment;

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
