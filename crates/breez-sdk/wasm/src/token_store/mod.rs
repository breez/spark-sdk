use macros::async_trait;
use platform_utils::time::{Instant, SystemTime};
use serde::{Deserialize, Serialize};
use spark_wallet::{
    GetTokenOutputsFilter, ReservationTarget, SelectionStrategy, TokenMetadata, TokenOutput,
    TokenOutputServiceError, TokenOutputStore, TokenOutputWithPrevOut, TokenOutputs,
    TokenOutputsPerStatus, TokenOutputsReservation, TokenOutputsReservationId,
    TokenReservationPurpose,
};
use tracing::info;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_futures::js_sys::Promise;

pub struct WasmTokenStore {
    token_store: TokenStoreJs,
}

impl WasmTokenStore {
    pub fn new(token_store: TokenStoreJs) -> Self {
        Self { token_store }
    }
}

// WASM is single-threaded
unsafe impl Send for WasmTokenStore {}
unsafe impl Sync for WasmTokenStore {}

fn js_error_to_token_error(js_error: JsValue) -> TokenOutputServiceError {
    let error_message = get_detailed_js_error(&js_error);
    if error_message.contains("InsufficientFunds") {
        TokenOutputServiceError::InsufficientFunds
    } else {
        TokenOutputServiceError::Generic(error_message)
    }
}

fn get_detailed_js_error(js_error: &JsValue) -> String {
    if js_error.is_instance_of::<js_sys::Error>() {
        let error = js_sys::Error::from(js_error.clone());
        let message = error.message();
        let name = error.name();
        return format!("JavaScript error: {} - {}", name, message);
    }

    if let Some(error_str) = js_error.as_string() {
        return format!("JavaScript error: {}", error_str);
    }

    if let Ok(json_str) = js_sys::JSON::stringify(js_error)
        && let Some(json) = json_str.as_string()
    {
        return format!("JavaScript error object: {}", json);
    }

    "JavaScript token store operation failed (Unknown error type)".to_string()
}

// ===== Serde helper types =====
//
// These types bridge between Rust domain types and JS objects.
// They use camelCase and String representations for PublicKey/u128
// to avoid serde_wasm_bindgen issues.

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WasmTokenMetadata {
    identifier: String,
    issuer_public_key: String,
    name: String,
    ticker: String,
    decimals: u32,
    max_supply: String,
    is_freezable: bool,
    creation_entity_public_key: Option<String>,
}

impl From<&TokenMetadata> for WasmTokenMetadata {
    fn from(m: &TokenMetadata) -> Self {
        Self {
            identifier: m.identifier.clone(),
            issuer_public_key: m.issuer_public_key.to_string(),
            name: m.name.clone(),
            ticker: m.ticker.clone(),
            decimals: m.decimals,
            max_supply: m.max_supply.to_string(),
            is_freezable: m.is_freezable,
            creation_entity_public_key: m.creation_entity_public_key.map(|pk| pk.to_string()),
        }
    }
}

impl TryFrom<WasmTokenMetadata> for TokenMetadata {
    type Error = TokenOutputServiceError;

    fn try_from(w: WasmTokenMetadata) -> Result<Self, Self::Error> {
        Ok(Self {
            identifier: w.identifier,
            issuer_public_key: w.issuer_public_key.parse().map_err(map_parse_err)?,
            name: w.name,
            ticker: w.ticker,
            decimals: w.decimals,
            max_supply: w.max_supply.parse().map_err(map_parse_err)?,
            is_freezable: w.is_freezable,
            creation_entity_public_key: w
                .creation_entity_public_key
                .map(|s| s.parse().map_err(map_parse_err))
                .transpose()?,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WasmTokenBalance {
    metadata: WasmTokenMetadata,
    balance: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WasmTokenOutput {
    id: String,
    owner_public_key: String,
    revocation_commitment: String,
    withdraw_bond_sats: u64,
    withdraw_relative_block_locktime: u64,
    token_public_key: Option<String>,
    token_identifier: String,
    token_amount: String,
}

impl From<&TokenOutput> for WasmTokenOutput {
    fn from(o: &TokenOutput) -> Self {
        Self {
            id: o.id.clone(),
            owner_public_key: o.owner_public_key.to_string(),
            revocation_commitment: o.revocation_commitment.clone(),
            withdraw_bond_sats: o.withdraw_bond_sats,
            withdraw_relative_block_locktime: o.withdraw_relative_block_locktime,
            token_public_key: o.token_public_key.map(|pk| pk.to_string()),
            token_identifier: o.token_identifier.clone(),
            token_amount: o.token_amount.to_string(),
        }
    }
}

impl TryFrom<WasmTokenOutput> for TokenOutput {
    type Error = TokenOutputServiceError;

    fn try_from(w: WasmTokenOutput) -> Result<Self, Self::Error> {
        Ok(Self {
            id: w.id,
            owner_public_key: w.owner_public_key.parse().map_err(map_parse_err)?,
            revocation_commitment: w.revocation_commitment,
            withdraw_bond_sats: w.withdraw_bond_sats,
            withdraw_relative_block_locktime: w.withdraw_relative_block_locktime,
            token_public_key: w
                .token_public_key
                .map(|s| s.parse().map_err(map_parse_err))
                .transpose()?,
            token_identifier: w.token_identifier,
            token_amount: w.token_amount.parse().map_err(map_parse_err)?,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WasmTokenOutputWithPrevOut {
    output: WasmTokenOutput,
    prev_tx_hash: String,
    prev_tx_vout: u32,
}

impl From<&TokenOutputWithPrevOut> for WasmTokenOutputWithPrevOut {
    fn from(o: &TokenOutputWithPrevOut) -> Self {
        Self {
            output: (&o.output).into(),
            prev_tx_hash: o.prev_tx_hash.clone(),
            prev_tx_vout: o.prev_tx_vout,
        }
    }
}

impl TryFrom<WasmTokenOutputWithPrevOut> for TokenOutputWithPrevOut {
    type Error = TokenOutputServiceError;

    fn try_from(w: WasmTokenOutputWithPrevOut) -> Result<Self, Self::Error> {
        Ok(Self {
            output: w.output.try_into()?,
            prev_tx_hash: w.prev_tx_hash,
            prev_tx_vout: w.prev_tx_vout,
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WasmTokenOutputs {
    metadata: WasmTokenMetadata,
    outputs: Vec<WasmTokenOutputWithPrevOut>,
}

impl From<&TokenOutputs> for WasmTokenOutputs {
    fn from(to: &TokenOutputs) -> Self {
        Self {
            metadata: (&to.metadata).into(),
            outputs: to.outputs.iter().map(Into::into).collect(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WasmTokenOutputsDeser {
    metadata: WasmTokenMetadata,
    outputs: Vec<WasmTokenOutputWithPrevOut>,
}

impl TryFrom<WasmTokenOutputsDeser> for TokenOutputs {
    type Error = TokenOutputServiceError;

    fn try_from(w: WasmTokenOutputsDeser) -> Result<Self, Self::Error> {
        Ok(Self {
            metadata: w.metadata.try_into()?,
            outputs: w
                .outputs
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WasmTokenOutputsPerStatus {
    metadata: WasmTokenMetadata,
    available: Vec<WasmTokenOutputWithPrevOut>,
    reserved_for_payment: Vec<WasmTokenOutputWithPrevOut>,
    reserved_for_swap: Vec<WasmTokenOutputWithPrevOut>,
}

impl TryFrom<WasmTokenOutputsPerStatus> for TokenOutputsPerStatus {
    type Error = TokenOutputServiceError;

    fn try_from(w: WasmTokenOutputsPerStatus) -> Result<Self, Self::Error> {
        Ok(Self {
            metadata: w.metadata.try_into()?,
            available: w
                .available
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?,
            reserved_for_payment: w
                .reserved_for_payment
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?,
            reserved_for_swap: w
                .reserved_for_swap
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WasmTokenOutputsReservation {
    id: String,
    token_outputs: WasmTokenOutputsDeser,
}

impl TryFrom<WasmTokenOutputsReservation> for TokenOutputsReservation {
    type Error = TokenOutputServiceError;

    fn try_from(w: WasmTokenOutputsReservation) -> Result<Self, Self::Error> {
        Ok(Self::new(w.id, w.token_outputs.try_into()?))
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum WasmGetTokenOutputsFilter {
    Identifier {
        identifier: String,
    },
    #[serde(rename_all = "camelCase")]
    IssuerPublicKey {
        issuer_public_key: String,
    },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum WasmReservationTarget {
    #[serde(rename_all = "camelCase")]
    MinTotalValue { value: String },
    #[serde(rename_all = "camelCase")]
    MaxOutputCount { value: u32 },
}

#[async_trait]
impl TokenOutputStore for WasmTokenStore {
    #[allow(clippy::cast_possible_truncation)]
    async fn set_tokens_outputs(
        &self,
        token_outputs: &[TokenOutputs],
        refresh_started_at: SystemTime,
    ) -> Result<(), TokenOutputServiceError> {
        let wasm_outputs: Vec<WasmTokenOutputs> = token_outputs.iter().map(Into::into).collect();
        let js_value = serde_wasm_bindgen::to_value(&wasm_outputs)
            .map_err(|e| TokenOutputServiceError::Generic(e.to_string()))?;
        // Convert SystemTime to milliseconds since epoch for JS
        let refresh_started_at_ms = refresh_started_at
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as f64;
        let promise = self
            .token_store
            .set_tokens_outputs(js_value, refresh_started_at_ms)
            .map_err(js_error_to_token_error)?;
        JsFuture::from(promise)
            .await
            .map_err(js_error_to_token_error)?;
        Ok(())
    }

    async fn get_token_balances(
        &self,
    ) -> Result<Vec<(TokenMetadata, u128)>, TokenOutputServiceError> {
        let promise = self
            .token_store
            .get_token_balances()
            .map_err(js_error_to_token_error)?;

        let t = Instant::now();
        let result = JsFuture::from(promise)
            .await
            .map_err(js_error_to_token_error)?;
        let js_dt = t.elapsed();

        let t = Instant::now();
        let wasm_balances: Vec<WasmTokenBalance> = serde_wasm_bindgen::from_value(result)
            .map_err(|e| TokenOutputServiceError::Generic(e.to_string()))?;
        let deser_dt = t.elapsed();

        info!(
            "WasmTokenStore::get_token_balances: {} entries, js_promise: {:?}, deserialize: {:?}",
            wasm_balances.len(),
            js_dt,
            deser_dt
        );

        wasm_balances
            .into_iter()
            .map(|b| {
                let metadata: TokenMetadata = b.metadata.try_into()?;
                let balance: u128 = b.balance.parse().map_err(map_parse_err)?;
                Ok((metadata, balance))
            })
            .collect()
    }

    async fn list_tokens_outputs(
        &self,
    ) -> Result<Vec<TokenOutputsPerStatus>, TokenOutputServiceError> {
        let promise = self
            .token_store
            .list_tokens_outputs()
            .map_err(js_error_to_token_error)?;

        let t = Instant::now();
        let result = JsFuture::from(promise)
            .await
            .map_err(js_error_to_token_error)?;
        let js_dt = t.elapsed();

        let t = Instant::now();
        let wasm_results: Vec<WasmTokenOutputsPerStatus> =
            serde_wasm_bindgen::from_value(result)
                .map_err(|e| TokenOutputServiceError::Generic(e.to_string()))?;
        let deser_dt = t.elapsed();

        info!(
            "WasmTokenStore::list_tokens_outputs: {} entries, js_promise: {:?}, deserialize: {:?}",
            wasm_results.len(),
            js_dt,
            deser_dt
        );

        wasm_results
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn get_token_outputs(
        &self,
        filter: GetTokenOutputsFilter<'_>,
    ) -> Result<TokenOutputsPerStatus, TokenOutputServiceError> {
        let wasm_filter = match filter {
            GetTokenOutputsFilter::Identifier(id) => WasmGetTokenOutputsFilter::Identifier {
                identifier: id.to_string(),
            },
            GetTokenOutputsFilter::IssuerPublicKey(pk) => {
                WasmGetTokenOutputsFilter::IssuerPublicKey {
                    issuer_public_key: pk.to_string(),
                }
            }
        };
        let filter_js = serde_wasm_bindgen::to_value(&wasm_filter)
            .map_err(|e| TokenOutputServiceError::Generic(e.to_string()))?;
        let promise = self
            .token_store
            .get_token_outputs(filter_js)
            .map_err(js_error_to_token_error)?;
        let result = JsFuture::from(promise)
            .await
            .map_err(js_error_to_token_error)?;
        let wasm_result: WasmTokenOutputsPerStatus = serde_wasm_bindgen::from_value(result)
            .map_err(|e| TokenOutputServiceError::Generic(e.to_string()))?;
        wasm_result.try_into()
    }

    async fn insert_token_outputs(
        &self,
        token_outputs: &TokenOutputs,
    ) -> Result<(), TokenOutputServiceError> {
        let wasm_outputs: WasmTokenOutputs = token_outputs.into();
        let js_value = serde_wasm_bindgen::to_value(&wasm_outputs)
            .map_err(|e| TokenOutputServiceError::Generic(e.to_string()))?;
        let promise = self
            .token_store
            .insert_token_outputs(js_value)
            .map_err(js_error_to_token_error)?;
        JsFuture::from(promise)
            .await
            .map_err(js_error_to_token_error)?;
        Ok(())
    }

    #[allow(clippy::cast_possible_truncation)]
    async fn reserve_token_outputs(
        &self,
        token_identifier: &str,
        target: ReservationTarget,
        purpose: TokenReservationPurpose,
        preferred_outputs: Option<Vec<TokenOutputWithPrevOut>>,
        selection_strategy: Option<SelectionStrategy>,
    ) -> Result<TokenOutputsReservation, TokenOutputServiceError> {
        let wasm_target = match target {
            ReservationTarget::MinTotalValue(v) => WasmReservationTarget::MinTotalValue {
                value: v.to_string(),
            },
            ReservationTarget::MaxOutputCount(c) => {
                WasmReservationTarget::MaxOutputCount { value: c as u32 }
            }
        };
        let target_js = serde_wasm_bindgen::to_value(&wasm_target)
            .map_err(|e| TokenOutputServiceError::Generic(e.to_string()))?;

        let purpose_str = match purpose {
            TokenReservationPurpose::Payment => "Payment",
            TokenReservationPurpose::Swap => "Swap",
        };

        let preferred_js = match preferred_outputs {
            Some(ref outputs) => {
                let wasm_outputs: Vec<WasmTokenOutputWithPrevOut> =
                    outputs.iter().map(Into::into).collect();
                serde_wasm_bindgen::to_value(&wasm_outputs)
                    .map_err(|e| TokenOutputServiceError::Generic(e.to_string()))?
            }
            None => JsValue::NULL,
        };

        let strategy_str = selection_strategy.map(|s| match s {
            SelectionStrategy::SmallestFirst => "SmallestFirst",
            SelectionStrategy::LargestFirst => "LargestFirst",
        });
        let strategy_js = match strategy_str {
            Some(s) => JsValue::from_str(s),
            None => JsValue::NULL,
        };

        let promise = self
            .token_store
            .reserve_token_outputs(
                token_identifier.to_string(),
                target_js,
                purpose_str.to_string(),
                preferred_js,
                strategy_js,
            )
            .map_err(js_error_to_token_error)?;
        let result = JsFuture::from(promise)
            .await
            .map_err(js_error_to_token_error)?;
        let wasm_reservation: WasmTokenOutputsReservation = serde_wasm_bindgen::from_value(result)
            .map_err(|e| TokenOutputServiceError::Generic(e.to_string()))?;
        wasm_reservation.try_into()
    }

    async fn cancel_reservation(
        &self,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError> {
        let promise = self
            .token_store
            .cancel_reservation(id.clone())
            .map_err(js_error_to_token_error)?;
        JsFuture::from(promise)
            .await
            .map_err(js_error_to_token_error)?;
        Ok(())
    }

    async fn finalize_reservation(
        &self,
        id: &TokenOutputsReservationId,
    ) -> Result<(), TokenOutputServiceError> {
        let promise = self
            .token_store
            .finalize_reservation(id.clone())
            .map_err(js_error_to_token_error)?;
        JsFuture::from(promise)
            .await
            .map_err(js_error_to_token_error)?;
        Ok(())
    }

    async fn now(&self) -> Result<SystemTime, TokenOutputServiceError> {
        let promise = self.token_store.now().map_err(js_error_to_token_error)?;
        let result = JsFuture::from(promise)
            .await
            .map_err(js_error_to_token_error)?;
        let ms = result.as_f64().ok_or_else(|| {
            TokenOutputServiceError::Generic("now() did not return a number".to_string())
        })?;
        let duration = std::time::Duration::from_millis(ms as u64);
        Ok(SystemTime::UNIX_EPOCH + duration)
    }
}

fn map_parse_err<E: std::fmt::Display>(e: E) -> TokenOutputServiceError {
    TokenOutputServiceError::Generic(format!("Parse error: {e}"))
}

// ===== TypeScript interface =====

#[wasm_bindgen(typescript_custom_section)]
const TOKEN_STORE_INTERFACE: &str = r#"
interface WasmTokenMetadata {
    identifier: string;
    issuerPublicKey: string;
    name: string;
    ticker: string;
    decimals: number;
    maxSupply: string;
    isFreezable: boolean;
    creationEntityPublicKey: string | null;
}

interface WasmTokenOutput {
    id: string;
    ownerPublicKey: string;
    revocationCommitment: string;
    withdrawBondSats: number;
    withdrawRelativeBlockLocktime: number;
    tokenPublicKey: string | null;
    tokenIdentifier: string;
    tokenAmount: string;
}

interface WasmTokenOutputWithPrevOut {
    output: WasmTokenOutput;
    prevTxHash: string;
    prevTxVout: number;
}

interface WasmTokenOutputs {
    metadata: WasmTokenMetadata;
    outputs: WasmTokenOutputWithPrevOut[];
}

interface WasmTokenOutputsPerStatus {
    metadata: WasmTokenMetadata;
    available: WasmTokenOutputWithPrevOut[];
    reservedForPayment: WasmTokenOutputWithPrevOut[];
    reservedForSwap: WasmTokenOutputWithPrevOut[];
}

interface WasmTokenOutputsReservation {
    id: string;
    tokenOutputs: WasmTokenOutputs;
}

interface WasmTokenBalance {
    metadata: WasmTokenMetadata;
    balance: string;
}

type WasmGetTokenOutputsFilter =
    | { type: 'identifier'; identifier: string }
    | { type: 'issuerPublicKey'; issuerPublicKey: string };

type WasmReservationTarget =
    | { type: 'minTotalValue'; value: string }
    | { type: 'maxOutputCount'; value: number };

export interface TokenStore {
    setTokensOutputs: (tokenOutputs: WasmTokenOutputs[], refreshStartedAtMs: number) => Promise<void>;
    listTokensOutputs: () => Promise<WasmTokenOutputsPerStatus[]>;
    getTokenBalances: () => Promise<WasmTokenBalance[]>;
    getTokenOutputs: (filter: WasmGetTokenOutputsFilter) => Promise<WasmTokenOutputsPerStatus>;
    insertTokenOutputs: (tokenOutputs: WasmTokenOutputs) => Promise<void>;
    reserveTokenOutputs: (
        tokenIdentifier: string,
        target: WasmReservationTarget,
        purpose: string,
        preferredOutputs: WasmTokenOutputWithPrevOut[] | null,
        selectionStrategy: string | null
    ) => Promise<WasmTokenOutputsReservation>;
    cancelReservation: (id: string) => Promise<void>;
    finalizeReservation: (id: string) => Promise<void>;
    now: () => Promise<number>;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "TokenStore")]
    pub type TokenStoreJs;

    #[wasm_bindgen(structural, method, js_name = setTokensOutputs, catch)]
    pub fn set_tokens_outputs(
        this: &TokenStoreJs,
        token_outputs: JsValue,
        refresh_started_at_ms: f64,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = listTokensOutputs, catch)]
    pub fn list_tokens_outputs(this: &TokenStoreJs) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = getTokenBalances, catch)]
    pub fn get_token_balances(this: &TokenStoreJs) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = getTokenOutputs, catch)]
    pub fn get_token_outputs(this: &TokenStoreJs, filter: JsValue) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = insertTokenOutputs, catch)]
    pub fn insert_token_outputs(
        this: &TokenStoreJs,
        token_outputs: JsValue,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = reserveTokenOutputs, catch)]
    pub fn reserve_token_outputs(
        this: &TokenStoreJs,
        token_identifier: String,
        target: JsValue,
        purpose: String,
        preferred_outputs: JsValue,
        selection_strategy: JsValue,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = cancelReservation, catch)]
    pub fn cancel_reservation(this: &TokenStoreJs, id: String) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = finalizeReservation, catch)]
    pub fn finalize_reservation(this: &TokenStoreJs, id: String) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = now, catch)]
    pub fn now(this: &TokenStoreJs) -> Result<Promise, JsValue>;
}
