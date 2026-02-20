use tokio::sync::Mutex;

use crate::{
    Config, MaxDepositClaimFeeUpdate, MaxFee, StableBalanceConfig, StableBalanceConfigUpdate,
    UpdateConfigRequest,
};

/// Indicates which config fields were changed by an update.
#[derive(Default)]
pub(crate) struct ConfigChanges {
    /// Whether `stable_balance_config` was changed (needs service lifecycle handling).
    pub stable_balance_config: bool,
}

/// All runtime-mutable configuration values.
#[derive(Clone, Debug)]
pub(crate) struct RuntimeConfig {
    pub stable_balance_config: Option<StableBalanceConfig>,
    pub max_deposit_claim_fee: Option<MaxFee>,
    pub prefer_spark_over_lightning: bool,
    pub sync_interval_secs: u32,
}

/// Manages runtime configuration behind a single mutex.
pub(crate) struct ConfigService {
    config: Mutex<RuntimeConfig>,
}

impl ConfigService {
    pub fn new(config: &Config) -> Self {
        Self {
            config: Mutex::new(RuntimeConfig {
                stable_balance_config: config.stable_balance_config.clone(),
                max_deposit_claim_fee: config.max_deposit_claim_fee.clone(),
                prefer_spark_over_lightning: config.prefer_spark_over_lightning,
                sync_interval_secs: config.sync_interval_secs,
            }),
        }
    }

    /// Returns the current stable balance auto-conversion config, if enabled.
    pub async fn stable_balance_config(&self) -> Option<StableBalanceConfig> {
        self.config.lock().await.stable_balance_config.clone()
    }

    /// Returns the maximum fee allowed for automatic deposit claims.
    pub async fn max_deposit_claim_fee(&self) -> Option<MaxFee> {
        self.config.lock().await.max_deposit_claim_fee.clone()
    }

    /// Returns whether Spark transfers are preferred over Lightning.
    pub async fn prefer_spark_over_lightning(&self) -> bool {
        self.config.lock().await.prefer_spark_over_lightning
    }

    /// Returns the interval in seconds between periodic wallet syncs.
    pub async fn sync_interval_secs(&self) -> u32 {
        self.config.lock().await.sync_interval_secs
    }

    /// Applies a partial config update. Only fields set to `Some` in the request
    /// are modified; `None` fields are left unchanged.
    ///
    /// Returns [`ConfigChanges`] indicating which fields were modified, so the
    /// caller can handle any required side-effects (e.g. service lifecycle).
    pub async fn update(&self, request: &UpdateConfigRequest) -> ConfigChanges {
        let mut config = self.config.lock().await;
        let mut changes = ConfigChanges::default();

        if let Some(ref update) = request.stable_balance_config {
            let new_val = match update {
                StableBalanceConfigUpdate::Set { config: c } => Some(c.clone()),
                StableBalanceConfigUpdate::Unset => None,
            };
            changes.stable_balance_config = true;
            config.stable_balance_config = new_val;
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

        changes
    }
}
