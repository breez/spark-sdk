//! Cross-version (backward-compatibility) test support for `spark-wallet`.
//!
//! Two builds of the wallet are linked into one binary: [`vold`] wraps the
//! previous SDK release (git tag pinned in `Cargo.toml`) and [`vnew`] wraps
//! the current workspace build. Tests start a flow with one version, go
//! offline, and finish it with the other version from the same seed: any
//! divergence in key derivation or signing between the two builds breaks the
//! continuation, so the signer is black-boxed end to end.
//!
//! All wallets run with background processing disabled and are driven
//! explicitly (`sync()` / `claim_deposit()` / `transfer()`): "offline" is the
//! default state, and there are no event races between the two builds.

use anyhow::Result;

pub use spark_itest::fixtures::bitcoind::BitcoindFixture;
pub use spark_itest::fixtures::setup::TestFixtures;
pub use spark_itest::helpers::wait_for;

/// Amount used for every deposit and transfer. Local fixtures run without an
/// SSP, so partial sends (which need a leaf swap for change) are not possible;
/// flows always move the full balance as a single leaf.
pub const FUND_SATS: u64 = 100_000;

pub fn random_seed() -> [u8; 32] {
    let mut seed = [0u8; 32];
    rand::Rng::fill(&mut rand::thread_rng(), &mut seed);
    seed
}

/// Finds the output index paying `address` in `tx`.
fn find_vout(tx: &bitcoin::Transaction, address: &bitcoin::Address) -> Result<u32> {
    for (vout, output) in tx.output.iter().enumerate() {
        if let Ok(out_address) =
            bitcoin::Address::from_script(&output.script_pubkey, bitcoin::Network::Regtest)
            && &out_address == address
        {
            return Ok(u32::try_from(vout)?);
        }
    }
    anyhow::bail!("deposit address not found in funding transaction outputs")
}

/// Generates the version-specific test API. Both modules expose the same
/// functions over their own `spark-wallet` build; only the crate path and the
/// signer wiring in `wallet()` differ (the old builder takes the low-level
/// `Signer` directly, the new one takes a `SparkSigner`).
macro_rules! version_module {
    ($name:ident, $krate:ident, $build_signer:item) => {
        pub mod $name {
            use std::str::FromStr;
            use std::sync::Arc;

            use anyhow::{Context, Result};
            use $krate::{
                LeafOptimizationOptions, Network, OperatorConfig, OperatorPoolConfig, PublicKey,
                RetryConfig, ServiceProviderConfig, SparkAddress, SparkWallet, SparkWalletConfig,
                TokenOutputsOptimizationOptions, WalletBuilder,
            };

            use crate::{BitcoindFixture, TestFixtures};

            $build_signer

            /// Wallet config pointing at the fixture operators; mirrors
            /// `TestFixtures::create_wallet_config` for this crate version.
            pub fn config(fx: &TestFixtures) -> Result<SparkWalletConfig> {
                let mut operator_configs = Vec::new();
                for operator in &fx.spark_so.operators {
                    operator_configs.push(OperatorConfig {
                        address: format!("https://127.0.0.1:{}", operator.host_port).parse()?,
                        ca_cert: Some(operator.ca_cert.as_bytes().to_vec()),
                        id: operator.index,
                        identifier: operator.identifier,
                        identity_public_key: operator.public_key,
                        user_agent: None,
                    });
                }

                Ok(SparkWalletConfig {
                    network: Network::Regtest,
                    operator_pool: OperatorPoolConfig::new(0, operator_configs)?,
                    split_secret_threshold: spark_itest::fixtures::spark_so::MIN_SIGNERS as u32,
                    reconnect_interval_seconds: 1,
                    service_provider_config: ServiceProviderConfig {
                        base_url: String::new(),
                        schema_endpoint: None,
                        identity_public_key: PublicKey::from_slice(&[2; 33])?,
                        user_agent: Some("spark-compat-itest/0.1.0".to_string()),
                        retry_config: RetryConfig::default(),
                    },
                    tokens_config: SparkWalletConfig::default_tokens_config(),
                    leaf_optimization_options: LeafOptimizationOptions::default(),
                    leaf_auto_optimize_enabled: false,
                    token_outputs_optimization_options: TokenOutputsOptimizationOptions {
                        min_outputs_threshold: 50,
                        target_output_count: 5,
                        auto_optimize_interval: None,
                    },
                    self_payment_allowed: false,
                    max_concurrent_claims: 1,
                })
            }

            /// Builds a wallet from `seed` with background processing disabled;
            /// drive it explicitly with the helpers below.
            pub async fn wallet(fx: &TestFixtures, seed: &[u8; 32]) -> Result<SparkWallet> {
                let spark_signer = build_signer(seed)?;
                Ok(WalletBuilder::new(config(fx)?, spark_signer)
                    .with_background_processing(false)
                    .build()
                    .await?)
            }

            pub fn address_string(wallet: &SparkWallet) -> Result<String> {
                Ok(wallet.get_spark_address()?.to_address_string()?)
            }

            /// Generates a deposit address and funds it on-chain WITHOUT
            /// claiming, returning the funding tx and vout for the other
            /// version to claim.
            pub async fn unclaimed_deposit(
                wallet: &SparkWallet,
                bitcoind: &BitcoindFixture,
            ) -> Result<(bitcoin::Transaction, u32)> {
                let address = wallet.generate_deposit_address().await?.address;
                let txid = bitcoind
                    .fund_address(&address, bitcoin::Amount::from_sat(crate::FUND_SATS))
                    .await?;
                let tx = bitcoind.get_transaction(&txid).await?;
                let vout = crate::find_vout(&tx, &address)?;
                Ok((tx, vout))
            }

            /// Claims a deposit created (possibly by the other version), mines
            /// the confirmation, and waits until the funds are spendable.
            pub async fn claim_deposit(
                wallet: &SparkWallet,
                bitcoind: &BitcoindFixture,
                tx: bitcoin::Transaction,
                vout: u32,
            ) -> Result<()> {
                let txid = tx.compute_txid();
                wallet.claim_deposit(tx, vout).await?;
                bitcoind.generate_blocks(1).await?;
                bitcoind.wait_for_tx_confirmation(&txid, 1).await?;
                await_balance(wallet, crate::FUND_SATS).await
            }

            /// Funds the wallet via a deposit it claims itself.
            pub async fn fund(wallet: &SparkWallet, bitcoind: &BitcoindFixture) -> Result<()> {
                let (tx, vout) = unclaimed_deposit(wallet, bitcoind).await?;
                claim_deposit(wallet, bitcoind, tx, vout).await
            }

            /// Sends the full balance to a bech32m Spark address string.
            pub async fn send_all(wallet: &SparkWallet, to: &str) -> Result<()> {
                let address = SparkAddress::from_str(to).context("parsing spark address")?;
                wallet.transfer(crate::FUND_SATS, &address, None).await?;
                Ok(())
            }

            /// Creates a Spark invoice for the full flow amount.
            pub async fn create_invoice(wallet: &SparkWallet) -> Result<String> {
                Ok(wallet
                    .create_spark_invoice(
                        Some(u128::from(crate::FUND_SATS)),
                        None,
                        None,
                        None,
                        None,
                    )
                    .await?)
            }

            /// Pays a Spark invoice (amount taken from the invoice).
            pub async fn pay_invoice(wallet: &SparkWallet, invoice: &str) -> Result<()> {
                wallet.fulfill_spark_invoice(invoice, None, None).await?;
                Ok(())
            }

            /// Polls `sync()` (which refreshes leaves and claims pending
            /// transfers) until the balance equals `expected`.
            pub async fn await_balance(wallet: &SparkWallet, expected: u64) -> Result<()> {
                crate::wait_for(
                    || async {
                        if wallet.sync().await.is_err() {
                            return false;
                        }
                        wallet.get_balance().await.is_ok_and(|b| b == expected)
                    },
                    60,
                    &format!("balance == {expected}"),
                )
                .await
            }
        }
    };
}

version_module!(
    vold,
    spark_wallet_old,
    /// The previous release's builder takes the low-level `Signer` directly.
    fn build_signer(seed: &[u8; 32]) -> Result<Arc<spark_wallet_old::DefaultSigner>> {
        Ok(Arc::new(spark_wallet_old::DefaultSigner::new(
            seed,
            spark_wallet_old::Network::Regtest,
        )?))
    }
);

version_module!(
    vnew,
    spark_wallet,
    /// The current builder takes the high-level `SparkSigner`; wrap the
    /// in-process signer in the adapter, as production does.
    fn build_signer(seed: &[u8; 32]) -> Result<Arc<spark_wallet::SparkSignerAdapter>> {
        Ok(Arc::new(spark_wallet::SparkSignerAdapter::new(Arc::new(
            spark_wallet::DefaultSigner::new(seed, spark_wallet::Network::Regtest)?,
        ))))
    }
);
