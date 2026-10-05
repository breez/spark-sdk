use breez_sdk_common::buy::{cashapp::CashAppProvider, moonpay::USDC_SOLANA_PRECISION};

use crate::{
    BuyBitcoinRequest, BuyBitcoinResponse, MoonpayDelivery, Network, ReceivePaymentMethod,
    ReceivePaymentRequest,
    cross_chain::{
        CrossChainFeeMode, CrossChainProvider, CrossChainReceiveInfo, CrossChainRouteFilter,
        CrossChainRoutePair, rescale_decimals,
    },
    error::SdkError,
};

use super::{BreezSdk, helpers::get_deposit_address};

const SOLANA_CHAIN: &str = "solana";
const USDC_ASSET: &str = "USDC";

/// Decimal places of `MoonpayDelivery::CrossChain::amount`.
const USD_AMOUNT_DECIMALS: u32 = 6;

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
impl BreezSdk {
    /// Initiates a Bitcoin purchase flow via an external provider.
    ///
    /// Returns a URL the user should open to complete the purchase.
    /// The request variant determines the provider and its parameters:
    ///
    /// - [`BuyBitcoinRequest::Moonpay`]: Fiat-to-Bitcoin, delivered to an
    ///   on-chain deposit address or as USDC on Solana (see [`MoonpayDelivery`]).
    /// - [`BuyBitcoinRequest::CashApp`]: Lightning invoice + `cash.app` deep link (mainnet only).
    pub async fn buy_bitcoin(
        &self,
        request: BuyBitcoinRequest,
    ) -> Result<BuyBitcoinResponse, SdkError> {
        let url = match request {
            BuyBitcoinRequest::Moonpay {
                delivery,
                redirect_url,
            } => match delivery.unwrap_or_default() {
                MoonpayDelivery::Bitcoin { amount_sat } => {
                    let address =
                        get_deposit_address(&self.spark_wallet, &self.storage, true).await?;
                    self.buy_bitcoin_provider
                        .buy_bitcoin(address, amount_sat, redirect_url)
                        .await
                        .map_err(|e| {
                            SdkError::Generic(format!("Failed to create buy bitcoin URL: {e}"))
                        })?
                }
                MoonpayDelivery::CrossChain { amount, fee_mode } => {
                    let (info, decimals) = self
                        .quote_moonpay_usdc_solana(
                            amount,
                            fee_mode.unwrap_or(CrossChainFeeMode::FeesExcluded),
                        )
                        .await?;
                    let url = self
                        .buy_bitcoin_provider
                        .buy_usdc_solana(
                            info.deposit_address.clone(),
                            info.deposit_amount,
                            u32::from(decimals),
                            redirect_url,
                        )
                        .await
                        .map_err(|e| {
                            SdkError::Generic(format!("Failed to create buy bitcoin URL: {e}"))
                        })?;
                    return Ok(BuyBitcoinResponse {
                        url,
                        cross_chain_info: Some(info),
                    });
                }
            },
            BuyBitcoinRequest::CashApp { amount_sats } => {
                if !matches!(self.config.network, Network::Mainnet) {
                    return Err(SdkError::Generic(
                        "CashApp is only available on mainnet".to_string(),
                    ));
                }
                if amount_sats == 0 {
                    return Err(SdkError::Generic(
                        "CashApp requires a non-zero amount".to_string(),
                    ));
                }
                let receive_response = self
                    .receive_bolt11_invoice(
                        "breez-onramp".to_string(),
                        Some(amount_sats),
                        None,
                        None,
                        None,
                    )
                    .await?;
                CashAppProvider::build_url(&receive_response.payment_request)
            }
        };

        Ok(BuyBitcoinResponse {
            url,
            cross_chain_info: None,
        })
    }
}

impl BreezSdk {
    /// Quotes a USDC-on-Solana receive for a `MoonPay` purchase to fund.
    /// `amount` is in USD at 6 decimals. Returns the quote, with its deposit
    /// rounded up to whole cents, and the route's decimals.
    async fn quote_moonpay_usdc_solana(
        &self,
        amount: u128,
        fee_mode: CrossChainFeeMode,
    ) -> Result<(CrossChainReceiveInfo, u8), SdkError> {
        if amount == 0 {
            return Err(SdkError::InvalidInput(
                "MoonPay cross-chain delivery requires a non-zero amount".to_string(),
            ));
        }
        let service = self
            .cross_chain_context
            .get(CrossChainProvider::Orchestra)
            .map_err(|_| {
                SdkError::InvalidInput(
                    "MoonPay cross-chain delivery requires cross_chain_config on mainnet"
                        .to_string(),
                )
            })?;
        let route = service
            .get_routes(&CrossChainRouteFilter::Receive {
                contract_address: None,
            })
            .await?
            .into_iter()
            .find(is_usdc_solana)
            .ok_or_else(|| SdkError::CrossChainRouteUnavailable {
                reason: "USDC on Solana is not offered".to_string(),
                temporary: false,
            })?;
        let decimals = route.decimals;

        // With fees included the amount is the deposit itself, so it has to
        // be whole cents already.
        let amount = match fee_mode {
            CrossChainFeeMode::FeesIncluded => {
                round_up_to_precision(amount, USD_AMOUNT_DECIMALS, USDC_SOLANA_PRECISION)
            }
            CrossChainFeeMode::FeesExcluded => amount,
        };
        let amount = rescale_decimals(amount, USD_AMOUNT_DECIMALS, u32::from(decimals))?;
        let response = self
            .receive_payment(ReceivePaymentRequest {
                payment_method: ReceivePaymentMethod::CrossChain {
                    route,
                    amount,
                    destination: None,
                    fee_mode: Some(fee_mode),
                    max_slippage_bps: None,
                    target_overpay_bps: None,
                },
            })
            .await?;
        let mut info = response.cross_chain_info.ok_or_else(|| {
            SdkError::Generic("Cross-chain receive returned no quote".to_string())
        })?;
        // MoonPay buys whole cents. The quote can call for a fraction of one.
        info.deposit_amount = round_up_to_precision(
            info.deposit_amount,
            u32::from(decimals),
            USDC_SOLANA_PRECISION,
        );
        Ok((info, decimals))
    }
}

fn is_usdc_solana(route: &CrossChainRoutePair) -> bool {
    route.chain.eq_ignore_ascii_case(SOLANA_CHAIN) && route.asset.eq_ignore_ascii_case(USDC_ASSET)
}

/// Rounds `amount` (base units at `decimals`) up to the nearest whole unit
/// at `precision` fractional digits.
fn round_up_to_precision(amount: u128, decimals: u32, precision: u32) -> u128 {
    let Some(shift) = decimals.checked_sub(precision) else {
        return amount;
    };
    let unit = 10u128.saturating_pow(shift);
    amount.div_ceil(unit).saturating_mul(unit)
}

#[cfg(test)]
mod tests {
    use super::round_up_to_precision;

    #[test]
    fn test_round_up_to_precision() {
        assert_eq!(round_up_to_precision(12_340_000, 6, 2), 12_340_000);
        assert_eq!(round_up_to_precision(12_340_001, 6, 2), 12_350_000);
        assert_eq!(round_up_to_precision(12_349_999, 6, 2), 12_350_000);
        assert_eq!(round_up_to_precision(0, 6, 2), 0);
        assert_eq!(round_up_to_precision(1_234, 2, 2), 1_234);
        assert_eq!(round_up_to_precision(5, 1, 2), 5);
    }
}
