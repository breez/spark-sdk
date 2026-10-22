use std::{collections::HashMap, time::Duration};

use anyhow::{Context, bail};
use base64::{Engine, prelude::BASE64_STANDARD};
use breez_sdk_spark::{
    BreezSdk, PayX402Request, X402Header, X402PaymentAmount, X402PaymentStatus, X402Request,
};
use clap::Subcommand;
use platform_utils::{HttpClient, HttpResponse, create_http_client};

use crate::command::print_value;

const PAYMENT_REQUIRED_HEADER: &str = "payment-required";
const PAYMENT_SIGNATURE_HEADER: &str = "payment-signature";
const PAYMENT_RESPONSE_HEADER: &str = "payment-response";
/// Times to pay the same challenge again while its payment is pending. Fetching
/// again instead would bring a new challenge with a new invoice to pay.
const PENDING_RETRIES: usize = 3;
/// Times to send the paid request again while the server answers 402 without a
/// new challenge, meaning the payment may still settle.
const SETTLE_RETRIES: usize = 5;
const SETTLE_RETRY_DELAY: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Subcommand)]
pub enum X402Command {
    /// Fetch a URL, paying its x402 Lightning challenge if the server asks for one
    Fetch {
        /// The URL to fetch
        url: String,
        /// The most to spend in sats, routing fee included
        #[arg(long)]
        max_amount_sats: u64,
        /// The HTTP method: GET or POST
        #[arg(short = 'X', long, default_value = "GET")]
        method: String,
        /// The request body
        #[arg(short = 'd', long)]
        body: Option<String>,
        /// A request header as "Name: value". Can be repeated
        #[arg(short = 'H', long = "header")]
        headers: Vec<String>,
    },
}

pub async fn handle_command(sdk: &BreezSdk, command: X402Command) -> Result<bool, anyhow::Error> {
    match command {
        X402Command::Fetch {
            url,
            max_amount_sats,
            method,
            body,
            headers,
        } => {
            let client = create_http_client(None)?;
            // The server hashes the headers it binds, so send every header
            // explicitly: anything the HTTP client added on its own would be
            // missing from the hash computed here.
            let mut headers = parse_headers(&headers)?;
            headers
                .entry("accept".to_string())
                .or_insert_with(|| "*/*".to_string());

            let response = send(&*client, &method, &url, &headers, body.clone()).await?;
            if response.status != 402 {
                print_response(&response);
                return Ok(true);
            }
            let payment_required = response
                .header(PAYMENT_REQUIRED_HEADER)
                .context("402 response has no PAYMENT-REQUIRED header")?
                .to_string();

            let x402 = sdk.get_x402();
            let request = PayX402Request {
                payment_required,
                request: X402Request::Http {
                    method: method.clone(),
                    url: url.clone(),
                    headers: headers
                        .iter()
                        .map(|(name, value)| X402Header {
                            name: name.clone(),
                            value: value.clone(),
                        })
                        .collect(),
                    body: body.as_ref().map(|b| b.as_bytes().to_vec()),
                },
                max_amount: X402PaymentAmount::Lightning {
                    amount_sats: max_amount_sats,
                },
                conversion_options: None,
                completion_timeout_secs: None,
            };
            let mut paid = x402.pay(request.clone()).await?;
            for _ in 0..PENDING_RETRIES {
                if paid.status == X402PaymentStatus::Paid {
                    break;
                }
                println!("Payment is pending, waiting for it to complete...");
                paid = x402.pay(request.clone()).await?;
            }
            print_value(&paid)?;
            let Some(signature) = paid.payment_signature else {
                println!("Payment is still pending. Check its status with list-payments.");
                return Ok(true);
            };

            headers.insert(PAYMENT_SIGNATURE_HEADER.to_string(), signature);
            let mut response = send(&*client, &method, &url, &headers, body.clone()).await?;
            for _ in 0..SETTLE_RETRIES {
                if response.status != 402 || response.header(PAYMENT_REQUIRED_HEADER).is_some() {
                    break;
                }
                println!("Payment is not settled yet, sending it again...");
                tokio::time::sleep(SETTLE_RETRY_DELAY).await;
                response = send(&*client, &method, &url, &headers, body.clone()).await?;
            }
            print_response(&response);
            Ok(true)
        }
    }
}

fn parse_headers(lines: &[String]) -> Result<HashMap<String, String>, anyhow::Error> {
    lines
        .iter()
        .map(|line| {
            let (name, value) = line
                .split_once(':')
                .with_context(|| format!("header {line:?} is not \"Name: value\""))?;
            Ok((name.trim().to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect()
}

async fn send(
    client: &dyn HttpClient,
    method: &str,
    url: &str,
    headers: &HashMap<String, String>,
    body: Option<String>,
) -> Result<HttpResponse, anyhow::Error> {
    let headers = Some(headers.clone());
    let response = match method {
        "GET" => client.get(url.to_string(), headers).await?,
        "POST" => client.post(url.to_string(), headers, body).await?,
        other => bail!("unsupported method {other}: use GET or POST"),
    };
    Ok(response)
}

fn print_response(response: &HttpResponse) {
    println!("HTTP {}", response.status);
    if let Some(settlement) = response.header(PAYMENT_RESPONSE_HEADER)
        && let Ok(json) = BASE64_STANDARD.decode(settlement)
    {
        println!("PAYMENT-RESPONSE: {}", String::from_utf8_lossy(&json));
    }
    println!("{}", response.body);
}
