use tokio::sync::watch;

use crate::{
    Config, MaxDepositClaimFeeUpdate, MaxFee, Network, StableBalanceConfig,
    StableBalanceConfigUpdate, UpdateConfigRequest,
};

/// All runtime-mutable configuration values.
#[derive(Clone, Debug)]
pub(crate) struct RuntimeConfig {
    pub stable_balance_config: Option<StableBalanceConfig>,
    pub max_deposit_claim_fee: Option<MaxFee>,
    pub prefer_spark_over_lightning: bool,
    pub sync_interval_secs: u32,
}

/// Manages SDK configuration: immutable fields with sync getters, and
/// runtime-mutable fields behind a `watch` channel that services can subscribe to.
pub(crate) struct ConfigService {
    network: Network,
    lnurl_domain: Option<String>,
    private_enabled_default: bool,

    /// Mutable config. Subscribers are notified on change.
    config_tx: watch::Sender<RuntimeConfig>,
}

impl ConfigService {
    pub fn new(config: &Config) -> Self {
        let runtime = RuntimeConfig {
            stable_balance_config: config.stable_balance_config.clone(),
            max_deposit_claim_fee: config.max_deposit_claim_fee.clone(),
            prefer_spark_over_lightning: config.prefer_spark_over_lightning,
            sync_interval_secs: config.sync_interval_secs,
        };
        let (config_tx, _) = watch::channel(runtime);

        Self {
            network: config.network,
            lnurl_domain: config.lnurl_domain.clone(),
            private_enabled_default: config.private_enabled_default,
            config_tx,
        }
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn lnurl_domain(&self) -> Option<&str> {
        self.lnurl_domain.as_deref()
    }

    pub fn private_enabled_default(&self) -> bool {
        self.private_enabled_default
    }

    /// Subscribe to runtime config changes
    pub fn subscribe(&self) -> watch::Receiver<RuntimeConfig> {
        self.config_tx.subscribe()
    }

    /// Returns the maximum fee allowed for automatic deposit claims.
    pub fn max_deposit_claim_fee(&self) -> Option<MaxFee> {
        self.config_tx.borrow().max_deposit_claim_fee.clone()
    }

    /// Returns whether Spark transfers are preferred over Lightning.
    pub fn prefer_spark_over_lightning(&self) -> bool {
        self.config_tx.borrow().prefer_spark_over_lightning
    }

    /// Returns the interval in seconds between periodic wallet syncs.
    pub fn sync_interval_secs(&self) -> u32 {
        self.config_tx.borrow().sync_interval_secs
    }

    /// Applies a partial config update. Only fields set to `Some` in the request
    /// are modified; `None` fields are left unchanged.
    ///
    /// Subscribers to config updates are notified automatically.
    pub fn update(&self, request: &UpdateConfigRequest) {
        self.config_tx.send_modify(|config| {
            if let Some(ref update) = request.stable_balance_config {
                config.stable_balance_config = match update {
                    StableBalanceConfigUpdate::Set { config: c } => Some(c.clone()),
                    StableBalanceConfigUpdate::Unset => None,
                };
            }

            if let Some(ref update) = request.max_deposit_claim_fee {
                match update {
                    MaxDepositClaimFeeUpdate::Set { fee } => {
                        config.max_deposit_claim_fee = Some(fee.clone());
                    }
                    MaxDepositClaimFeeUpdate::Unset => config.max_deposit_claim_fee = None,
                }
            }

            if let Some(v) = request.prefer_spark_over_lightning {
                config.prefer_spark_over_lightning = v;
            }

            if let Some(v) = request.sync_interval_secs {
                config.sync_interval_secs = v;
            }
        });
    }
}
