//! What a test runs against: the deployed regtest, or a stack of its own.
//!
//! A test takes one [`Environment`] and creates every wallet it needs from it, so
//! the wallets of a test share operators, an SSP and a faucet, whichever the build
//! runs against.

use std::str::FromStr;

use anyhow::Result;
use bitcoin::{Transaction, Txid};
use breez_sdk_spark::{
    Config, LeafOptimizationConfig, MaxFee, Network, default_config, default_server_config,
};
use rstest::fixture;
use spark_itest::mempool::MempoolClient;

use crate::SdkInstance;
use crate::faucet::{FaucetConfig, RegtestFaucet};
use crate::fixtures::{random_mnemonic, stable_balance_config};
use crate::helpers::regtest::{
    build_sdk_with_custom_config, build_sdk_with_dir, build_sdk_with_external_signer,
};
use crate::local_sdk::{LocalIdentity, LocalStack};

/// The environment this build's tests run against.
#[fixture]
pub async fn env() -> Result<Environment> {
    Environment::new().await
}

pub enum Environment {
    /// The shared regtest the SDK is released against.
    Deployed,
    /// This test's own operators, sspd and bitcoind.
    Local(Box<LocalStack>),
}

impl Environment {
    /// The environment this build runs against: a local stack under `local-itest`,
    /// the deployed regtest otherwise.
    pub async fn new() -> Result<Self> {
        #[cfg(feature = "local-itest")]
        {
            Ok(Self::Local(Box::new(LocalStack::start().await?)))
        }
        #[cfg(not(feature = "local-itest"))]
        {
            Ok(Self::Deployed)
        }
    }

    /// The faucet that funds this environment's wallets.
    pub fn faucet(&self) -> Result<RegtestFaucet> {
        RegtestFaucet::with_config(self.faucet_config())
    }

    fn faucet_config(&self) -> FaucetConfig {
        match self {
            Environment::Deployed => FaucetConfig::default(),
            Environment::Local(stack) => FaucetConfig::for_ssp(&stack.ssp_base_url()),
        }
    }

    /// The transaction `txid`, as this environment's chain has it.
    pub async fn transaction(&self, txid: &str) -> Result<Transaction> {
        match self {
            Environment::Deployed => MempoolClient::new()?.get_transaction(txid).await,
            Environment::Local(stack) => {
                stack
                    .fixtures()
                    .bitcoind
                    .get_transaction(&Txid::from_str(txid)?)
                    .await
            }
        }
    }

    /// A wallet with a fresh identity and temporary storage.
    pub async fn create_wallet(&self) -> Result<SdkInstance> {
        match self {
            Environment::Deployed => {
                let dir = tempfile::Builder::new()
                    .prefix("breez-sdk-wallet")
                    .tempdir()?;
                let path = dir.path().to_string_lossy().to_string();
                build_sdk_with_dir(path, rand::random(), Some(dir)).await
            }
            Environment::Local(stack) => {
                stack
                    .create_wallet(LocalIdentity::Seed(rand::random()), false, |_cfg| {})
                    .await
            }
        }
    }

    /// A wallet with `configure` applied to its config last.
    pub async fn create_wallet_with(
        &self,
        configure: impl FnOnce(&mut Config) + Send,
    ) -> Result<SdkInstance> {
        self.create_configured_wallet(false, configure).await
    }

    /// A wallet that runs no background tasks, as a server deployment does.
    pub async fn create_server_wallet_with(
        &self,
        configure: impl FnOnce(&mut Config) + Send,
    ) -> Result<SdkInstance> {
        self.create_configured_wallet(true, configure).await
    }

    /// A wallet whose keys live behind the external signer interface.
    pub async fn create_external_signer_wallet(&self) -> Result<SdkInstance> {
        match self {
            Environment::Deployed => {
                let dir = tempfile::Builder::new()
                    .prefix("breez-sdk-ext-signer")
                    .tempdir()?;
                let path = dir.path().to_string_lossy().to_string();
                build_sdk_with_external_signer(path, random_mnemonic()?, Some(dir)).await
            }
            Environment::Local(stack) => {
                stack
                    .create_wallet(
                        LocalIdentity::ExternalMnemonic(random_mnemonic()?),
                        false,
                        |_cfg| {},
                    )
                    .await
            }
        }
    }

    /// A wallet that refuses no deposit claim fee.
    pub async fn create_wallet_without_claim_fee_ceiling(&self) -> Result<SdkInstance> {
        self.create_wallet_with(|cfg| cfg.max_deposit_claim_fee = None)
            .await
    }

    /// A wallet that refuses any deposit claim fee, so nothing claims for it.
    pub async fn create_wallet_refusing_claim_fees(&self) -> Result<SdkInstance> {
        self.create_wallet_with(|cfg| cfg.max_deposit_claim_fee = Some(MaxFee::Fixed { amount: 0 }))
            .await
    }

    /// A wallet whose leaf optimization only runs when the test asks for it, with a
    /// target multiplicity high enough that the planner produces real swaps.
    pub async fn create_wallet_optimizing_on_demand(&self) -> Result<SdkInstance> {
        self.create_wallet_with(manual_optimization).await
    }

    /// [`Self::create_wallet_optimizing_on_demand`] on the server runtime.
    pub async fn create_server_wallet_optimizing_on_demand(&self) -> Result<SdkInstance> {
        self.create_server_wallet_with(manual_optimization).await
    }

    /// A wallet whose leaf set changes only through what the test does.
    pub async fn create_wallet_without_auto_optimization(&self) -> Result<SdkInstance> {
        self.create_wallet_with(|cfg| cfg.leaf_optimization_config.auto_enabled = false)
            .await
    }

    /// A wallet holding its balance in a stable token.
    pub async fn create_wallet_with_stable_balance(&self) -> Result<SdkInstance> {
        self.create_wallet_with(|cfg| cfg.stable_balance_config = Some(stable_balance_config()))
            .await
    }

    async fn create_configured_wallet(
        &self,
        server_mode: bool,
        configure: impl FnOnce(&mut Config) + Send,
    ) -> Result<SdkInstance> {
        match self {
            Environment::Deployed => {
                let dir = tempfile::Builder::new()
                    .prefix("breez-sdk-wallet")
                    .tempdir()?;
                let path = dir.path().to_string_lossy().to_string();
                let mut config = if server_mode {
                    default_server_config(Network::Regtest)
                } else {
                    default_config(Network::Regtest)
                };
                configure(&mut config);
                build_sdk_with_custom_config(path, rand::random(), config, Some(dir), true).await
            }
            Environment::Local(stack) => {
                stack
                    .create_wallet(LocalIdentity::Seed(rand::random()), server_mode, configure)
                    .await
            }
        }
    }
}

fn manual_optimization(config: &mut Config) {
    config.leaf_optimization_config = LeafOptimizationConfig {
        auto_enabled: false,
        multiplicity: 15,
    };
}
