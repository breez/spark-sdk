#[cfg(test)]
mod tests;

use macros::async_trait;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_futures::js_sys::Promise;

use crate::models::{DepositInfo, Payment, PaymentMetadata, PaymentStatus, UpdateDepositPayload};

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

#[async_trait]
impl breez_sdk_spark::Storage for WasmStorage {
    async fn get_cached_item(
        &self,
        key: String,
    ) -> Result<Option<String>, breez_sdk_spark::StorageError> {
        let promise = self
            .storage
            .get_cached_item(key)
            .map_err(js_error_to_storage_error)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_storage_error)?;

        if result.is_null() || result.is_undefined() {
            Ok(None)
        } else {
            Ok(result.as_string())
        }
    }

    async fn set_cached_item(
        &self,
        key: String,
        value: String,
    ) -> Result<(), breez_sdk_spark::StorageError> {
        let promise = self
            .storage
            .set_cached_item(key, value)
            .map_err(js_error_to_storage_error)?;
        let future = JsFuture::from(promise);
        future.await.map_err(js_error_to_storage_error)?;
        Ok(())
    }

    async fn list_payments(
        &self,
        offset: Option<u32>,
        limit: Option<u32>,
        status: Option<breez_sdk_spark::PaymentStatus>,
    ) -> Result<Vec<breez_sdk_spark::Payment>, breez_sdk_spark::StorageError> {
        let promise = self
            .storage
            .list_payments(offset, limit, status.map(|s| s.into()))
            .map_err(js_error_to_storage_error)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_storage_error)?;

        let payments: Vec<Payment> = serde_wasm_bindgen::from_value(result)
            .map_err(|e| breez_sdk_spark::StorageError::Serialization(e.to_string()))?;
        Ok(payments.into_iter().map(|p| p.into()).collect())
    }

    async fn insert_payment(
        &self,
        payment: breez_sdk_spark::Payment,
    ) -> Result<(), breez_sdk_spark::StorageError> {
        let promise = self
            .storage
            .insert_payment(payment.into())
            .map_err(js_error_to_storage_error)?;
        let future = JsFuture::from(promise);
        future.await.map_err(js_error_to_storage_error)?;
        Ok(())
    }

    async fn set_payment_metadata(
        &self,
        payment_id: String,
        metadata: breez_sdk_spark::PaymentMetadata,
    ) -> Result<(), breez_sdk_spark::StorageError> {
        let metadata: PaymentMetadata = metadata.clone().into();
        let promise = self
            .storage
            .set_payment_metadata(payment_id, metadata)
            .map_err(js_error_to_storage_error)?;
        let future = JsFuture::from(promise);
        future.await.map_err(js_error_to_storage_error)?;
        Ok(())
    }

    async fn get_payment_by_id(
        &self,
        id: String,
    ) -> Result<breez_sdk_spark::Payment, breez_sdk_spark::StorageError> {
        let promise = self
            .storage
            .get_payment_by_id(id)
            .map_err(js_error_to_storage_error)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_storage_error)?;

        let payment: Payment = serde_wasm_bindgen::from_value(result)
            .map_err(|e| breez_sdk_spark::StorageError::Serialization(e.to_string()))?;
        Ok(payment.into())
    }

    async fn add_deposit(
        &self,
        txid: String,
        vout: u32,
        amount_sats: u64,
    ) -> Result<(), breez_sdk_spark::StorageError> {
        let promise = self
            .storage
            .add_deposit(txid, vout, amount_sats)
            .map_err(js_error_to_storage_error)?;
        let future = JsFuture::from(promise);
        future.await.map_err(js_error_to_storage_error)?;
        Ok(())
    }

    async fn delete_deposit(
        &self,
        txid: String,
        vout: u32,
    ) -> Result<(), breez_sdk_spark::StorageError> {
        let promise = self
            .storage
            .delete_deposit(txid, vout)
            .map_err(js_error_to_storage_error)?;
        let future = JsFuture::from(promise);
        future.await.map_err(js_error_to_storage_error)?;
        Ok(())
    }

    async fn list_deposits(
        &self,
    ) -> Result<Vec<breez_sdk_spark::DepositInfo>, breez_sdk_spark::StorageError> {
        let promise = self
            .storage
            .list_deposits()
            .map_err(js_error_to_storage_error)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_storage_error)?;

        let deposits: Vec<DepositInfo> = serde_wasm_bindgen::from_value(result)
            .map_err(|e| breez_sdk_spark::StorageError::Serialization(e.to_string()))?;
        Ok(deposits.into_iter().map(|d| d.into()).collect())
    }

    async fn update_deposit(
        &self,
        txid: String,
        vout: u32,
        payload: breez_sdk_spark::UpdateDepositPayload,
    ) -> Result<(), breez_sdk_spark::StorageError> {
        let promise = self
            .storage
            .update_deposit(txid, vout, payload.into())
            .map_err(js_error_to_storage_error)?;
        let future = JsFuture::from(promise);
        future.await.map_err(js_error_to_storage_error)?;
        Ok(())
    }
}

#[wasm_bindgen(typescript_custom_section)]
const STORAGE_INTERFACE: &'static str = r#"export interface Storage {
    getCachedItem: (key: string) => Promise<string | null>;
    setCachedItem: (key: string, value: string) => Promise<void>;
    listPayments: (offset?: number, limit?: number) => Promise<Payment[]>;
    insertPayment: (payment: Payment) => Promise<void>;
    setPaymentMetadata: (paymentId: string, metadata: PaymentMetadata) => Promise<void>;
    getPaymentById: (id: string) => Promise<Payment>;
    addDeposit: (txid: string, vout: number, amount_sats: number) => Promise<void>;
    deleteDeposit: (txid: string, vout: number) => Promise<void>;
    listDeposits: () => Promise<DepositInfo[]>;
    updateDeposit: (txid: string, vout: number, payload: UpdateDepositPayload) => Promise<void>;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "Storage")]
    pub type Storage;

    #[wasm_bindgen(structural, method, js_name = getCachedItem, catch)]
    pub fn get_cached_item(this: &Storage, key: String) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = setCachedItem, catch)]
    pub fn set_cached_item(this: &Storage, key: String, value: String) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = listPayments, catch)]
    pub fn list_payments(
        this: &Storage,
        offset: Option<u32>,
        limit: Option<u32>,
        status: Option<PaymentStatus>,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = insertPayment, catch)]
    pub fn insert_payment(this: &Storage, payment: Payment) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = setPaymentMetadata, catch)]
    pub fn set_payment_metadata(
        this: &Storage,
        payment_id: String,
        metadata: PaymentMetadata,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = getPaymentById, catch)]
    pub fn get_payment_by_id(this: &Storage, id: String) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = addDeposit, catch)]
    pub fn add_deposit(
        this: &Storage,
        txid: String,
        vout: u32,
        amount_sats: u64,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = deleteDeposit, catch)]
    pub fn delete_deposit(this: &Storage, txid: String, vout: u32) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = listDeposits, catch)]
    pub fn list_deposits(this: &Storage) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = updateDeposit, catch)]
    pub fn update_deposit(
        this: &Storage,
        txid: String,
        vout: u32,
        payload: UpdateDepositPayload,
    ) -> Result<Promise, JsValue>;
}
