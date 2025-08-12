use std::str::FromStr;

use bitcoin::{
    Transaction, Txid,
    consensus::encode::{deserialize_hex, serialize_hex},
};
use reqwest::header::CONTENT_TYPE;

use crate::config::Config;

pub async fn get_transaction(
    config: &Config,
    txid: String,
) -> Result<Transaction, Box<dyn std::error::Error>> {
    let url = format!("{}/tx/{}/hex", config.mempool_url, txid);
    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .basic_auth(
            config.mempool_username.clone(),
            Some(config.mempool_password.clone()),
        )
        .send()
        .await?;
    let hex = response.text().await?;
    let tx = deserialize_hex(&hex)?;
    Ok(tx)
}

pub async fn broadcast_transaction(
    config: &Config,
    tx: Transaction,
) -> Result<Txid, Box<dyn std::error::Error>> {
    let tx_hex = serialize_hex(&tx);
    let url = format!("{}/tx", config.mempool_url);
    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .basic_auth(
            config.mempool_username.clone(),
            Some(config.mempool_password.clone()),
        )
        .header(CONTENT_TYPE, "text/plain")
        .body(tx_hex.clone())
        .send()
        .await?;
    let text = response.text().await?;
    let txid = Txid::from_str(&text).map_err(|_| {
        println!("Refund tx hex: {tx_hex}");
        format!("Failed to parse txid from response: {text}")
    })?;
    Ok(txid)
}
