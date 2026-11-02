use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context, bail};
use base64::{Engine, prelude::BASE64_STANDARD};
use breez_sdk_spark::{
    BreezSdk, PayX402Request, X402Header, X402PaymentAmount, X402PaymentStatus, X402Request,
};
use clap::Subcommand;
use platform_utils::{
    HttpClient, HttpResponse, RedirectFilter, create_http_client_with_redirect_filter,
};
use url::Url;

const PAYMENT_REQUIRED_HEADER: &str = "payment-required";
const PAYMENT_SIGNATURE_HEADER: &str = "payment-signature";
const PAYMENT_RESPONSE_HEADER: &str = "payment-response";
/// Times to pay the same challenge again while its payment is pending. Fetching
/// again instead would bring a new challenge with a new invoice to pay.
const PENDING_RETRIES: usize = 3;
/// Times to send the paid request again while the server answers that the
/// payment may still settle.
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
            if method == "GET" && body.is_some() {
                bail!("a GET request has no body: use -X POST to send one");
            }
            // Redirects aren't followed: the payment then commits to exactly
            // the request sent.
            let refuse: RedirectFilter =
                Arc::new(|_, _| Err("x402 fetch doesn't follow redirects".to_string()));
            let client = create_http_client_with_redirect_filter(None, None, refuse)?;
            // The server hashes the headers it binds, so send every header
            // explicitly: anything the HTTP client added on its own would be
            // missing from the hash computed here.
            let mut headers = parse_headers(&headers)?;
            headers
                .entry("accept".to_string())
                .or_insert_with(|| "*/*".to_string());
            let url = as_sent(&url)?;

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
            // The payment signature proves the payment, so it isn't printed.
            println!(
                "x402 payment {} is {:?}: {} sats, fee {} sats",
                paid.payment.id, paid.status, paid.payment.amount, paid.payment.fees
            );
            let Some(signature) = paid.payment_signature else {
                println!("Payment is still pending. Check its status with list-payments.");
                return Ok(true);
            };

            headers.insert(PAYMENT_SIGNATURE_HEADER.to_string(), signature);
            let mut response = send(&*client, &method, &url, &headers, body.clone()).await?;
            for _ in 0..SETTLE_RETRIES {
                if !settlement_pending(&response) {
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

/// The URL as the HTTP client sends it: `http://host` goes out as
/// `http://host/`, and a fragment never goes out.
fn as_sent(url: &str) -> Result<String, anyhow::Error> {
    let mut url = Url::parse(url)?;
    url.set_fragment(None);
    Ok(url.to_string())
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

/// Whether the server answered that the payment may still settle: a 402 with
/// no new challenge and the core x402 specification's non-final reason.
fn settlement_pending(response: &HttpResponse) -> bool {
    response.status == 402
        && response.header(PAYMENT_REQUIRED_HEADER).is_none()
        && response
            .header(PAYMENT_RESPONSE_HEADER)
            .and_then(|header| BASE64_STANDARD.decode(header).ok())
            .and_then(|json| serde_json::from_slice::<serde_json::Value>(&json).ok())
            .is_some_and(|settlement| settlement["errorReason"] == "settlement_pending")
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

#[cfg(test)]
mod tests {
    use axum::{Router, response::Redirect, routing::get};

    use super::*;

    #[tokio::test]
    async fn refuses_to_follow_a_redirect() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/from", get(|| async { Redirect::temporary("/to") }))
            .route("/to", get(|| async { "ok" }));
        let server = platform_utils::spawn(async move { axum::serve(listener, app).await });

        let refuse: RedirectFilter = Arc::new(|_, _| Err("refused".to_string()));
        let client = create_http_client_with_redirect_filter(None, None, refuse).unwrap();
        let headers = HashMap::new();
        let error = send(&*client, "GET", &format!("{origin}/from"), &headers, None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("/to"), "{error}");
        server.abort();
    }

    #[test]
    fn sends_again_only_while_settlement_is_pending() {
        let response = |status, headers: &[(&str, &str)]| HttpResponse {
            status,
            body: String::new(),
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
                .collect(),
        };
        let reason =
            |reason: &str| BASE64_STANDARD.encode(format!(r#"{{"errorReason":"{reason}"}}"#));
        let pending = reason("settlement_pending");
        let duplicate = reason("duplicate_settlement");
        assert!(settlement_pending(&response(
            402,
            &[(PAYMENT_RESPONSE_HEADER, &pending)]
        )));
        for refused in [
            response(402, &[(PAYMENT_RESPONSE_HEADER, &duplicate)]),
            response(402, &[]),
            response(
                402,
                &[
                    (PAYMENT_RESPONSE_HEADER, &pending),
                    (PAYMENT_REQUIRED_HEADER, "challenge"),
                ],
            ),
            response(500, &[(PAYMENT_RESPONSE_HEADER, &pending)]),
        ] {
            assert!(!settlement_pending(&refused));
        }
    }

    #[test]
    fn binds_the_url_as_sent() {
        assert_eq!(
            as_sent("http://example.com#part").unwrap(),
            "http://example.com/"
        );
        assert_eq!(
            as_sent("https://example.com/a?b=1").unwrap(),
            "https://example.com/a?b=1"
        );
    }
}
