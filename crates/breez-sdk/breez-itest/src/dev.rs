//! Lightspark's dev environment, where the SSP prices payments with partner fees.
//!
//! Its endpoints are bundled, and its credentials are read from the environment
//! so they stay out of the repository. A test that finds no partner key skips:
//!
//! | Variable | Holds |
//! |---|---|
//! | `DEV_PARTNER_KEY` | path to a file holding the partner's P-256 PKCS#8 PEM key, alone or among other text |
//! | `DEV_FAUCET_USERNAME`, `DEV_FAUCET_PASSWORD` | the dev faucet's credentials |
//! | `DEV_SPARK_CONFIG` | optional, path to a `SparkConfig` JSON to use instead of the bundled dev operators and SSP |
//! | `DEV_CHAIN_SERVICE_URL`, `DEV_FAUCET_URL` | optional, to override the dev mempool API and faucet |
//! | `DEV_PARTNER_ISSUER`, `DEV_PARTNER_AFFILIATE` | the token's `iss` and `sub` (dev fixture values by default) |
//! | `DEV_AFFILIATE_MNEMONIC`, `DEV_PARTNER_MNEMONIC` | optional, the fee payout wallets, to check their shares |
//! | `DEV_FUNDING_MNEMONIC` | optional, a funded wallet tests are paid from instead of the rate-limited faucet |

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use base64::Engine;
use breez_sdk_spark::{
    ChainApiType, GetInfoRequest, Network, NewRestChainServiceRequest, PaymentRequest,
    PrepareSendPaymentRequest, ReceivePaymentMethod, ReceivePaymentRequest, SdkBuilder, Seed,
    SendPaymentRequest, SparkConfig, default_config, new_rest_chain_service,
};
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use p256::pkcs8::DecodePrivateKey;
use rand::RngCore;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::helpers::regtest::apply_storage;
use crate::{ChannelEventListener, FaucetConfig, RegtestFaucet, SdkInstance, wait_for_balance};

/// The dev operators and SSP.
const DEV_SPARK_CONFIG: &str = include_str!("dev_spark_config.json");
const DEV_CHAIN_SERVICE_URL: &str = "https://regtest-mempool.dev.dev.sparkinfra.net/api";

/// The dev environment, or `None` when it isn't configured.
pub struct DevEnv {
    pub spark_config: SparkConfig,
    pub chain_service_url: String,
    pub faucet: FaucetConfig,
    partner_key: SigningKey,
    issuer: String,
    affiliate: String,
    pub affiliate_mnemonic: Option<String>,
    pub partner_mnemonic: Option<String>,
    funding_mnemonic: Option<String>,
}

/// How to build a dev wallet.
#[derive(Default)]
pub struct DevWalletOptions {
    /// Attach a partner JWT, so the wallet quotes its Lightning payments.
    pub partner_jwt: bool,
    /// Restore this wallet instead of creating a new one.
    pub mnemonic: Option<String>,
    pub lnurl_domain: Option<String>,
    pub prefer_spark_over_lightning: bool,
    /// Reach the SSP here instead, such as through an
    /// [`crate::fixtures::ssp_fault::SspFaultProxy`].
    pub ssp_base_url: Option<String>,
}

impl DevEnv {
    /// The dev environment from the variables above. `None`, logged, when any
    /// required one is unset, so a test can skip.
    pub fn from_env() -> Option<Self> {
        match Self::load() {
            Ok(env) => Some(env),
            Err(e) => {
                warn!("Skipping a dev environment test: {e:#}");
                None
            }
        }
    }

    fn load() -> Result<Self> {
        let var = |name: &str| std::env::var(name).with_context(|| format!("{name} is not set"));
        let spark_config: SparkConfig = match std::env::var("DEV_SPARK_CONFIG") {
            Ok(path) => serde_json::from_slice(
                &std::fs::read(&path).with_context(|| format!("reading {path}"))?,
            )
            .with_context(|| format!("parsing {path}"))?,
            Err(_) => serde_json::from_str(DEV_SPARK_CONFIG)?,
        };
        let key_path = var("DEV_PARTNER_KEY")?;
        let key_file =
            std::fs::read_to_string(&key_path).with_context(|| format!("reading {key_path}"))?;
        let pem = pem_block(&key_file).context("no PKCS#8 private key in DEV_PARTNER_KEY")?;
        Ok(Self {
            chain_service_url: std::env::var("DEV_CHAIN_SERVICE_URL")
                .unwrap_or_else(|_| DEV_CHAIN_SERVICE_URL.to_string()),
            faucet: FaucetConfig {
                url: std::env::var("DEV_FAUCET_URL").unwrap_or_else(|_| {
                    format!("{}/graphql/spark/rc", spark_config.ssp_config.base_url)
                }),
                username: std::env::var("DEV_FAUCET_USERNAME").ok(),
                password: std::env::var("DEV_FAUCET_PASSWORD").ok(),
            },
            spark_config,
            partner_key: SigningKey::from_pkcs8_pem(pem)
                .map_err(|e| anyhow::anyhow!("invalid partner key: {e}"))?,
            issuer: std::env::var("DEV_PARTNER_ISSUER")
                .unwrap_or_else(|_| "breez.dev.test".to_string()),
            affiliate: std::env::var("DEV_PARTNER_AFFILIATE")
                .unwrap_or_else(|_| "breez.affiliate.dev.test".to_string()),
            affiliate_mnemonic: std::env::var("DEV_AFFILIATE_MNEMONIC").ok(),
            partner_mnemonic: std::env::var("DEV_PARTNER_MNEMONIC").ok(),
            funding_mnemonic: std::env::var("DEV_FUNDING_MNEMONIC").ok(),
        })
    }

    /// A partner JWT for the SSP and the operators, valid for an hour.
    pub fn partner_jwt(&self) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let encode = |value: serde_json::Value| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.to_string())
        };
        let signing_input = format!(
            "{}.{}",
            encode(serde_json::json!({"alg": "ES256", "typ": "JWT"})),
            encode(serde_json::json!({
                "iss": self.issuer,
                "sub": self.affiliate,
                "aud": ["spark-so", "spark-ssp"],
                "iat": now,
                "exp": now + 3600,
            })),
        );
        let signature: Signature = self.partner_key.sign(signing_input.as_bytes());
        format!(
            "{signing_input}.{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature.to_bytes())
        )
    }

    /// A connected and synced wallet on the dev environment.
    pub async fn wallet(&self, options: DevWalletOptions) -> Result<SdkInstance> {
        let temp_dir = tempfile::Builder::new().prefix("breez-dev").tempdir()?;
        let storage_dir = temp_dir.path().to_string_lossy().to_string();

        let mut config = default_config(Network::Regtest);
        config.api_key = None;
        let mut spark_config = self.spark_config.clone();
        if let Some(base_url) = options.ssp_base_url {
            spark_config.ssp_config.base_url = base_url;
        }
        config.spark_config = Some(spark_config);
        config.prefer_spark_over_lightning = options.prefer_spark_over_lightning;
        config.lnurl_domain = options.lnurl_domain;
        config.sync_interval_secs = 5;
        config.real_time_sync_server_url = None;

        let seed = match options.mnemonic {
            Some(mnemonic) => Seed::Mnemonic {
                mnemonic,
                passphrase: None,
            },
            None => {
                let mut entropy = [0u8; 32];
                rand::thread_rng().fill_bytes(&mut entropy);
                Seed::Entropy(entropy.to_vec())
            }
        };
        let chain_service = new_rest_chain_service(
            self.chain_service_url.clone(),
            Network::Regtest,
            ChainApiType::MempoolSpace,
            None,
            NewRestChainServiceRequest::default(),
        )
        .await?;
        let mut builder = SdkBuilder::new(config, seed).with_chain_service(chain_service);
        if options.partner_jwt {
            builder = builder.with_partner_jwt(self.partner_jwt());
        }
        let sdk = apply_storage(builder, storage_dir).await?.build().await?;

        let (tx, events) = mpsc::channel(100);
        sdk.add_event_listener(Box::new(ChannelEventListener { tx }))
            .await;
        sdk.get_info(GetInfoRequest {
            ensure_synced: Some(true),
        })
        .await?;

        Ok(SdkInstance {
            sdk,
            events,
            span: tracing::Span::current(),
            temp_dir: Some(temp_dir),
            data_sync_fixture: None,
            lnurl_fixture: None,
            faucet: self.faucet.clone(),
            turnkey_guard: None,
        })
    }

    /// Funds `wallet` with `amount_sats` over Spark, from the funding wallet if
    /// one is set and the dev faucet otherwise, and waits until it holds them.
    pub async fn fund(&self, wallet: &SdkInstance, amount_sats: u64) -> Result<()> {
        let initial = wallet
            .sdk
            .get_info(GetInfoRequest {
                ensure_synced: Some(false),
            })
            .await?
            .balance_sats;
        let address = wallet
            .sdk
            .receive_payment(ReceivePaymentRequest {
                payment_method: ReceivePaymentMethod::SparkAddress,
            })
            .await?
            .payment_request;
        if let Some(mnemonic) = &self.funding_mnemonic {
            let funder = self
                .wallet(DevWalletOptions {
                    mnemonic: Some(mnemonic.clone()),
                    ..Default::default()
                })
                .await?;
            let prepare_response = funder
                .sdk
                .prepare_send_payment(PrepareSendPaymentRequest {
                    payment_request: PaymentRequest::Input { input: address },
                    amount: Some(u128::from(amount_sats)),
                    token_identifier: None,
                    conversion_options: None,
                    fee_policy: None,
                })
                .await?;
            funder
                .sdk
                .send_payment(SendPaymentRequest {
                    prepare_response,
                    options: None,
                    idempotency_key: None,
                })
                .await?;
            info!("Funding wallet sent {amount_sats} sats");
        } else {
            let faucet = Arc::new(RegtestFaucet::with_config(self.faucet.clone())?);
            let transfer = faucet.fund_address(&address, amount_sats).await?;
            info!("Dev faucet sent {amount_sats} sats in {transfer}");
        }
        wait_for_balance(&wallet.sdk, Some(initial + amount_sats), None, 120).await?;
        Ok(())
    }
}

/// The first PKCS#8 private key PEM block in `text`.
fn pem_block(text: &str) -> Option<&str> {
    const BEGIN: &str = "-----BEGIN PRIVATE KEY-----";
    const END: &str = "-----END PRIVATE KEY-----";
    let start = text.find(BEGIN)?;
    let end = start + text[start..].find(END)? + END.len();
    Some(&text[start..end])
}
