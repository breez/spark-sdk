#[cfg(test)]
mod tests;

use wasm_bindgen::prelude::*;

use crate::models::{DepositInfo, Payment, PaymentMetadata, UpdateDepositPayload};

pub struct WasmStorage {
    pub storage: Storage,
}

/// Helper function to convert JS exceptions to StorageError
fn js_error_to_storage_error(js_error: JsValue) -> breez_sdk_spark::StorageError {
    let error_message = js_error
        .as_string()
        .unwrap_or_else(|| "JavaScript storage operation failed".to_string());
    breez_sdk_spark::StorageError::Implementation(error_message)
}

// This assumes that we'll always be running in a single thread (true for Wasm environments)
unsafe impl Send for WasmStorage {}
unsafe impl Sync for WasmStorage {}

impl breez_sdk_spark::Storage for WasmStorage {
    fn get_cached_item(
        &self,
        key: String,
    ) -> Result<Option<String>, breez_sdk_spark::StorageError> {
        self.storage
            .get_cached_item(key)
            .map_err(js_error_to_storage_error)
    }

    fn set_cached_item(
        &self,
        key: String,
        value: String,
    ) -> Result<(), breez_sdk_spark::StorageError> {
        self.storage
            .set_cached_item(key, value)
            .map_err(js_error_to_storage_error)
    }

    fn list_payments(
        &self,
        offset: Option<u32>,
        limit: Option<u32>,
    ) -> Result<Vec<breez_sdk_spark::Payment>, breez_sdk_spark::StorageError> {
        let payments = self
            .storage
            .list_payments(offset, limit)
            .map_err(js_error_to_storage_error)?;
        Ok(payments.into_iter().map(|p| p.into()).collect())
    }

    fn insert_payment(
        &self,
        payment: breez_sdk_spark::Payment,
    ) -> Result<(), breez_sdk_spark::StorageError> {
        self.storage
            .insert_payment(payment.into())
            .map_err(js_error_to_storage_error)
    }

    fn set_payment_metadata(
        &self,
        payment_id: String,
        metadata: breez_sdk_spark::PaymentMetadata,
    ) -> Result<(), breez_sdk_spark::StorageError> {
        let metadata: PaymentMetadata = metadata.clone().into();
        self.storage
            .set_payment_metadata(payment_id, metadata)
            .map_err(js_error_to_storage_error)
    }

    fn get_payment_by_id(
        &self,
        id: String,
    ) -> Result<breez_sdk_spark::Payment, breez_sdk_spark::StorageError> {
        let payment = self
            .storage
            .get_payment_by_id(id)
            .map_err(js_error_to_storage_error)?;
        Ok(payment.into())
    }

    fn add_deposit(
        &self,
        txid: String,
        vout: u32,
        amount_sats: u64,
    ) -> Result<(), breez_sdk_spark::StorageError> {
        self.storage
            .add_deposit(txid, vout, amount_sats)
            .map_err(js_error_to_storage_error)
    }

    fn delete_deposit(&self, txid: String, vout: u32) -> Result<(), breez_sdk_spark::StorageError> {
        self.storage
            .delete_deposit(txid, vout)
            .map_err(js_error_to_storage_error)
    }

    fn list_deposits(
        &self,
    ) -> Result<Vec<breez_sdk_spark::DepositInfo>, breez_sdk_spark::StorageError> {
        let deposits = self
            .storage
            .list_deposits()
            .map_err(js_error_to_storage_error)?;
        Ok(deposits.into_iter().map(|d| d.into()).collect())
    }

    fn update_deposit(
        &self,
        txid: String,
        vout: u32,
        payload: breez_sdk_spark::UpdateDepositPayload,
    ) -> Result<(), breez_sdk_spark::StorageError> {
        self.storage
            .update_deposit(txid, vout, payload.into())
            .map_err(js_error_to_storage_error)
    }
}

#[wasm_bindgen(typescript_custom_section)]
const STORAGE_INTERFACE: &'static str = r#"export interface Storage {
    getCachedItem: (key: string) => string | null;
    setCachedItem: (key: string, value: string) => void;
    listPayments: (offset?: number, limit?: number) => Payment[];
    insertPayment: (payment: Payment) => void;
    setPaymentMetadata: (paymentId: string, metadata: PaymentMetadata) => void;
    getPaymentById: (id: string) => Payment;
    addDeposit: (txid: string, vout: number, amount_sats: number) => void;
    deleteDeposit: (txid: string, vout: number) => void;
    listDeposits: () => DepositInfo[];
    updateDeposit: (txid: string, vout: number, payload: UpdateDepositPayload) => void;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "Storage")]
    pub type Storage;

    #[wasm_bindgen(structural, method, js_name = getCachedItem, catch)]
    pub fn get_cached_item(this: &Storage, key: String) -> Result<Option<String>, JsValue>;

    #[wasm_bindgen(structural, method, js_name = setCachedItem, catch)]
    pub fn set_cached_item(this: &Storage, key: String, value: String) -> Result<(), JsValue>;

    #[wasm_bindgen(structural, method, js_name = listPayments, catch)]
    pub fn list_payments(
        this: &Storage,
        offset: Option<u32>,
        limit: Option<u32>,
    ) -> Result<Vec<Payment>, JsValue>;

    #[wasm_bindgen(structural, method, js_name = insertPayment, catch)]
    pub fn insert_payment(this: &Storage, payment: Payment) -> Result<(), JsValue>;

    #[wasm_bindgen(structural, method, js_name = setPaymentMetadata, catch)]
    pub fn set_payment_metadata(
        this: &Storage,
        payment_id: String,
        metadata: PaymentMetadata,
    ) -> Result<(), JsValue>;

    #[wasm_bindgen(structural, method, js_name = getPaymentById, catch)]
    pub fn get_payment_by_id(this: &Storage, id: String) -> Result<Payment, JsValue>;

    #[wasm_bindgen(structural, method, js_name = addDeposit, catch)]
    pub fn add_deposit(
        this: &Storage,
        txid: String,
        vout: u32,
        amount_sats: u64,
    ) -> Result<(), JsValue>;

    #[wasm_bindgen(structural, method, js_name = deleteDeposit, catch)]
    pub fn delete_deposit(this: &Storage, txid: String, vout: u32) -> Result<(), JsValue>;

    #[wasm_bindgen(structural, method, js_name = listDeposits, catch)]
    pub fn list_deposits(this: &Storage) -> Result<Vec<DepositInfo>, JsValue>;

    #[wasm_bindgen(structural, method, js_name = updateDeposit, catch)]
    pub fn update_deposit(
        this: &Storage,
        txid: String,
        vout: u32,
        payload: UpdateDepositPayload,
    ) -> Result<(), JsValue>;
}
