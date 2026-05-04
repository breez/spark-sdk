#[cfg(all(test, not(feature = "browser-tests")))]
mod tests;

use std::sync::Arc;

use macros::async_trait;
use platform_utils::time::Instant;
use platform_utils::tokio::sync::watch;
use serde::{Deserialize, Serialize};
use spark_wallet::{
    Leaves, LeavesReservation, LeavesReservationId, ReservationPurpose, ReserveResult,
    TargetAmounts, TreeNode, TreeServiceError, TreeStore,
};
use tracing::info;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_futures::js_sys::Promise;

pub struct WasmTreeStore {
    tree_store: TreeStoreJs,
    balance_changed_tx: Arc<watch::Sender<()>>,
    balance_changed_rx: watch::Receiver<()>,
}

impl WasmTreeStore {
    pub fn new(tree_store: TreeStoreJs) -> Self {
        let (balance_changed_tx, balance_changed_rx) = watch::channel(());
        Self {
            tree_store,
            balance_changed_tx: Arc::new(balance_changed_tx),
            balance_changed_rx,
        }
    }

    fn notify_balance_change(&self) {
        let _ = self.balance_changed_tx.send(());
    }
}

fn js_error_to_tree_error(js_error: JsValue) -> TreeServiceError {
    let error_message = get_detailed_js_error(&js_error);
    if error_message.contains("NonReservableLeaves") {
        TreeServiceError::NonReservableLeaves
    } else {
        TreeServiceError::Generic(error_message)
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

    "JavaScript tree store operation failed (Unknown error type)".to_string()
}

// WASM is single-threaded
unsafe impl Send for WasmTreeStore {}
unsafe impl Sync for WasmTreeStore {}

// ===== Deserialization types for JS results =====

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WasmLeaves {
    available: Vec<TreeNode>,
    not_available: Vec<TreeNode>,
    available_missing_from_operators: Vec<TreeNode>,
    reserved_for_payment: Vec<TreeNode>,
    reserved_for_swap: Vec<TreeNode>,
}

impl From<WasmLeaves> for Leaves {
    fn from(w: WasmLeaves) -> Self {
        Leaves {
            available: w.available,
            not_available: w.not_available,
            available_missing_from_operators: w.available_missing_from_operators,
            reserved_for_payment: w.reserved_for_payment,
            reserved_for_swap: w.reserved_for_swap,
        }
    }
}

#[derive(Deserialize)]
struct WasmLeavesReservation {
    id: String,
    leaves: Vec<TreeNode>,
}

impl From<WasmLeavesReservation> for LeavesReservation {
    fn from(w: WasmLeavesReservation) -> Self {
        LeavesReservation::new(w.leaves, w.id)
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum WasmReserveResult {
    Success {
        reservation: WasmLeavesReservation,
    },
    InsufficientFunds,
    WaitForPending {
        needed: u64,
        available: u64,
        pending: u64,
    },
}

impl From<WasmReserveResult> for ReserveResult {
    fn from(w: WasmReserveResult) -> Self {
        match w {
            WasmReserveResult::Success { reservation } => {
                ReserveResult::Success(reservation.into())
            }
            WasmReserveResult::InsufficientFunds => ReserveResult::InsufficientFunds,
            WasmReserveResult::WaitForPending {
                needed,
                available,
                pending,
            } => ReserveResult::WaitForPending {
                needed,
                available,
                pending,
            },
        }
    }
}

// ===== Serialization types for JS calls =====

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum WasmTargetAmounts {
    AmountAndFee {
        #[serde(rename = "amountSats")]
        amount_sats: u64,
        #[serde(rename = "feeSats")]
        fee_sats: Option<u64>,
    },
    ExactDenominations {
        denominations: Vec<u64>,
    },
}

impl From<&TargetAmounts> for WasmTargetAmounts {
    fn from(t: &TargetAmounts) -> Self {
        match t {
            TargetAmounts::AmountAndFee {
                amount_sats,
                fee_sats,
            } => WasmTargetAmounts::AmountAndFee {
                amount_sats: *amount_sats,
                fee_sats: *fee_sats,
            },
            TargetAmounts::ExactDenominations { denominations } => {
                WasmTargetAmounts::ExactDenominations {
                    denominations: denominations.clone(),
                }
            }
        }
    }
}

#[async_trait]
impl TreeStore for WasmTreeStore {
    async fn add_leaves(&self, leaves: &[TreeNode]) -> Result<(), TreeServiceError> {
        let leaves_js = serde_wasm_bindgen::to_value(leaves)
            .map_err(|e| TreeServiceError::Generic(e.to_string()))?;
        let promise = self
            .tree_store
            .add_leaves(leaves_js)
            .map_err(js_error_to_tree_error)?;
        JsFuture::from(promise)
            .await
            .map_err(js_error_to_tree_error)?;
        self.notify_balance_change();
        Ok(())
    }

    async fn get_available_balance(&self) -> Result<u64, TreeServiceError> {
        let promise = self
            .tree_store
            .get_available_balance()
            .map_err(js_error_to_tree_error)?;

        let t = Instant::now();
        let result = JsFuture::from(promise)
            .await
            .map_err(js_error_to_tree_error)?;
        let js_dt = t.elapsed();

        let balance = if let Some(n) = result.as_f64() {
            n as u64
        } else if let Ok(big) = result
            .clone()
            .dyn_into::<wasm_bindgen_futures::js_sys::BigInt>()
        {
            u64::try_from(big)
                .map_err(|e| TreeServiceError::Generic(format!("BigInt overflow: {e:?}")))?
        } else {
            return Err(TreeServiceError::Generic(
                "getAvailableBalance returned non-numeric value".to_string(),
            ));
        };

        info!("WasmTreeStore::get_available_balance: {balance}, js_promise: {js_dt:?}");
        Ok(balance)
    }

    async fn get_leaves(&self) -> Result<Leaves, TreeServiceError> {
        let promise = self
            .tree_store
            .get_leaves()
            .map_err(js_error_to_tree_error)?;

        let t = Instant::now();
        let result = JsFuture::from(promise)
            .await
            .map_err(js_error_to_tree_error)?;
        let js_dt = t.elapsed();

        let t = Instant::now();
        let wasm_leaves: WasmLeaves = serde_wasm_bindgen::from_value(result)
            .map_err(|e| TreeServiceError::Generic(e.to_string()))?;
        let deser_dt = t.elapsed();

        let leaves: Leaves = wasm_leaves.into();
        let count = leaves.available.len()
            + leaves.not_available.len()
            + leaves.available_missing_from_operators.len()
            + leaves.reserved_for_payment.len()
            + leaves.reserved_for_swap.len();
        info!(
            "WasmTreeStore::get_leaves: {} leaves, js_promise: {:?}, deserialize: {:?}",
            count, js_dt, deser_dt
        );
        Ok(leaves)
    }

    async fn set_leaves(
        &self,
        leaves: &[TreeNode],
        missing_operators_leaves: &[TreeNode],
        refresh_started_at: platform_utils::time::SystemTime,
    ) -> Result<(), TreeServiceError> {
        let leaves_js = serde_wasm_bindgen::to_value(leaves)
            .map_err(|e| TreeServiceError::Generic(e.to_string()))?;
        let missing_js = serde_wasm_bindgen::to_value(missing_operators_leaves)
            .map_err(|e| TreeServiceError::Generic(e.to_string()))?;

        let refresh_ms = refresh_started_at
            .duration_since(platform_utils::time::SystemTime::UNIX_EPOCH)
            .map_err(|e| TreeServiceError::Generic(e.to_string()))?
            .as_millis() as f64;

        let promise = self
            .tree_store
            .set_leaves(leaves_js, missing_js, refresh_ms)
            .map_err(js_error_to_tree_error)?;
        JsFuture::from(promise)
            .await
            .map_err(js_error_to_tree_error)?;
        self.notify_balance_change();
        Ok(())
    }

    async fn cancel_reservation(
        &self,
        id: &LeavesReservationId,
        leaves_to_keep: &[TreeNode],
    ) -> Result<(), TreeServiceError> {
        let leaves_js = serde_wasm_bindgen::to_value(leaves_to_keep)
            .map_err(|e| TreeServiceError::Generic(e.to_string()))?;
        let promise = self
            .tree_store
            .cancel_reservation(id.clone(), leaves_js)
            .map_err(js_error_to_tree_error)?;
        JsFuture::from(promise)
            .await
            .map_err(js_error_to_tree_error)?;
        self.notify_balance_change();
        Ok(())
    }

    async fn finalize_reservation(
        &self,
        id: &LeavesReservationId,
        new_leaves: Option<&[TreeNode]>,
    ) -> Result<(), TreeServiceError> {
        let new_leaves_js = match new_leaves {
            Some(leaves) => serde_wasm_bindgen::to_value(leaves)
                .map_err(|e| TreeServiceError::Generic(e.to_string()))?,
            None => JsValue::NULL,
        };
        let promise = self
            .tree_store
            .finalize_reservation(id.clone(), new_leaves_js)
            .map_err(js_error_to_tree_error)?;
        JsFuture::from(promise)
            .await
            .map_err(js_error_to_tree_error)?;
        self.notify_balance_change();
        Ok(())
    }

    async fn try_reserve_leaves(
        &self,
        target_amounts: Option<&TargetAmounts>,
        exact_only: bool,
        purpose: ReservationPurpose,
    ) -> Result<ReserveResult, TreeServiceError> {
        let total_start = Instant::now();
        let target_js = match target_amounts {
            Some(t) => {
                let wasm_target: WasmTargetAmounts = t.into();
                serde_wasm_bindgen::to_value(&wasm_target)
                    .map_err(|e| TreeServiceError::Generic(e.to_string()))?
            }
            None => JsValue::NULL,
        };
        let promise = self
            .tree_store
            .try_reserve_leaves(target_js, exact_only, purpose.to_string())
            .map_err(js_error_to_tree_error)?;
        let result = JsFuture::from(promise)
            .await
            .map_err(js_error_to_tree_error)?;
        let wasm_result: WasmReserveResult = serde_wasm_bindgen::from_value(result)
            .map_err(|e| TreeServiceError::Generic(e.to_string()))?;
        let reserve_result: ReserveResult = wasm_result.into();
        if matches!(&reserve_result, ReserveResult::Success(_)) {
            self.notify_balance_change();
        }
        let outcome = match &reserve_result {
            ReserveResult::Success(r) => format!("success(leaves={})", r.leaves.len()),
            ReserveResult::WaitForPending { .. } => "waitForPending".to_string(),
            ReserveResult::InsufficientFunds => "insufficientFunds".to_string(),
        };
        info!(
            "WasmTreeStore::try_reserve_leaves: {} (exact_only={}, purpose={:?}, took {:?})",
            outcome,
            exact_only,
            purpose,
            total_start.elapsed()
        );
        Ok(reserve_result)
    }

    async fn now(&self) -> Result<platform_utils::time::SystemTime, TreeServiceError> {
        let promise = self.tree_store.now().map_err(js_error_to_tree_error)?;
        let result = JsFuture::from(promise)
            .await
            .map_err(js_error_to_tree_error)?;
        let ms = result
            .as_f64()
            .ok_or_else(|| TreeServiceError::Generic("now() did not return a number".into()))?;
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let duration = std::time::Duration::from_millis(ms as u64);
        Ok(platform_utils::time::SystemTime::UNIX_EPOCH + duration)
    }

    fn subscribe_balance_changes(&self) -> watch::Receiver<()> {
        self.balance_changed_rx.clone()
    }

    async fn update_reservation(
        &self,
        reservation_id: &LeavesReservationId,
        reserved_leaves: &[TreeNode],
        change_leaves: &[TreeNode],
    ) -> Result<LeavesReservation, TreeServiceError> {
        let reserved_js = serde_wasm_bindgen::to_value(reserved_leaves)
            .map_err(|e| TreeServiceError::Generic(e.to_string()))?;
        let change_js = serde_wasm_bindgen::to_value(change_leaves)
            .map_err(|e| TreeServiceError::Generic(e.to_string()))?;
        let promise = self
            .tree_store
            .update_reservation(reservation_id.clone(), reserved_js, change_js)
            .map_err(js_error_to_tree_error)?;
        let result = JsFuture::from(promise)
            .await
            .map_err(js_error_to_tree_error)?;
        let wasm_reservation: WasmLeavesReservation = serde_wasm_bindgen::from_value(result)
            .map_err(|e| TreeServiceError::Generic(e.to_string()))?;
        self.notify_balance_change();
        Ok(wasm_reservation.into())
    }
}

// ===== TypeScript interface =====

#[wasm_bindgen(typescript_custom_section)]
const TREE_STORE_INTERFACE: &str = r#"
/** Serialized tree node. Key fields used by store implementations: id, status, value. */
interface TreeNode {
    id: string;
    tree_id: string;
    value: number;
    status: string;
    [key: string]: unknown;
}

interface Leaves {
    available: TreeNode[];
    notAvailable: TreeNode[];
    availableMissingFromOperators: TreeNode[];
    reservedForPayment: TreeNode[];
    reservedForSwap: TreeNode[];
}

interface LeavesReservation {
    id: string;
    leaves: TreeNode[];
}

type TargetAmounts =
    | { type: 'amountAndFee'; amountSats: number; feeSats: number | null }
    | { type: 'exactDenominations'; denominations: number[] };

type ReserveResult =
    | { type: 'success'; reservation: LeavesReservation }
    | { type: 'insufficientFunds' }
    | { type: 'waitForPending'; needed: number; available: number; pending: number };

export interface TreeStore {
    addLeaves: (leaves: TreeNode[]) => Promise<void>;
    getLeaves: () => Promise<Leaves>;
    getAvailableBalance: () => Promise<bigint | number>;
    setLeaves: (leaves: TreeNode[], missingLeaves: TreeNode[], refreshStartedAtMs: number) => Promise<void>;
    cancelReservation: (id: string, leavesToKeep: TreeNode[]) => Promise<void>;
    finalizeReservation: (id: string, newLeaves: TreeNode[] | null) => Promise<void>;
    tryReserveLeaves: (targetAmounts: TargetAmounts | null, exactOnly: boolean, purpose: string) => Promise<ReserveResult>;
    now: () => Promise<number>;
    updateReservation: (reservationId: string, reservedLeaves: TreeNode[], changeLeaves: TreeNode[]) => Promise<LeavesReservation>;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "TreeStore")]
    pub type TreeStoreJs;

    #[wasm_bindgen(structural, method, js_name = addLeaves, catch)]
    pub fn add_leaves(this: &TreeStoreJs, leaves: JsValue) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = getLeaves, catch)]
    pub fn get_leaves(this: &TreeStoreJs) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = getAvailableBalance, catch)]
    pub fn get_available_balance(this: &TreeStoreJs) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = setLeaves, catch)]
    pub fn set_leaves(
        this: &TreeStoreJs,
        leaves: JsValue,
        missing_leaves: JsValue,
        refresh_started_at_ms: f64,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = cancelReservation, catch)]
    pub fn cancel_reservation(
        this: &TreeStoreJs,
        id: String,
        leaves_to_keep: JsValue,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = finalizeReservation, catch)]
    pub fn finalize_reservation(
        this: &TreeStoreJs,
        id: String,
        new_leaves: JsValue,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = tryReserveLeaves, catch)]
    pub fn try_reserve_leaves(
        this: &TreeStoreJs,
        target_amounts: JsValue,
        exact_only: bool,
        purpose: String,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = now, catch)]
    pub fn now(this: &TreeStoreJs) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = updateReservation, catch)]
    pub fn update_reservation(
        this: &TreeStoreJs,
        reservation_id: String,
        reserved_leaves: JsValue,
        change_leaves: JsValue,
    ) -> Result<Promise, JsValue>;
}
