#[cfg(test)]
mod tests;

use breez_sdk_spark::{StorageError, sync_storage::SyncStorageError};
use macros::async_trait;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_futures::js_sys::Promise;

use crate::models::{
    DepositInfo, IncomingChange, ListPaymentsRequest, OutgoingChange, Payment, PaymentMetadata,
    Record, UnversionedRecordChange, UpdateDepositPayload,
};

pub struct WasmStorage {
    pub storage: Storage,
}

/// Helper function to convert JS exceptions to StorageError with detailed error logging
fn js_error_to_storage_error(js_error: JsValue) -> StorageError {
    let error_message = get_detailed_js_error(&js_error);
    StorageError::Implementation(error_message)
}

/// Helper function to convert JS exceptions to StorageError with detailed error logging
fn js_error_to_sync_storage_error(js_error: JsValue) -> SyncStorageError {
    let error_message = get_detailed_js_error(&js_error);
    SyncStorageError::Implementation(error_message)
}

/// Extract detailed error information from a JavaScript error value
fn get_detailed_js_error(js_error: &JsValue) -> String {
    // Check for DomException which is common for IndexedDB errors
    if js_error.is_instance_of::<web_sys::DomException>() {
        let dom_exception = web_sys::DomException::from(js_error.clone());
        let name = dom_exception.name();
        let message = dom_exception.message();
        let code = dom_exception.code();

        return format!("IndexedDB error: {} - {} (code: {})", name, message, code);
    }

    // Try to extract error as a JavaScript Error object
    if js_error.is_instance_of::<js_sys::Error>() {
        let error = js_sys::Error::from(js_error.clone());
        let message = error.message();
        let name = error.name();

        // Attempt to get the stack trace via toString() which often includes it
        let error_str = js_error
            .clone()
            .dyn_into::<js_sys::Object>()
            .map(|obj| obj.to_string().as_string().unwrap_or_default())
            .unwrap_or_default();

        return format!(
            "JavaScript error: {} - {} (Details: {})",
            name, message, error_str
        );
    }

    // If it's a string, use that directly
    if let Some(error_str) = js_error.as_string() {
        return format!("JavaScript error: {}", error_str);
    }

    // For any other type of error value, try to stringify it
    if let Ok(json_str) = js_sys::JSON::stringify(js_error)
        && let Some(json) = json_str.as_string()
    {
        return format!("JavaScript error object: {}", json);
    }

    // Fallback for when nothing else works
    "JavaScript storage operation failed (Unknown error type)".to_string()
}

// This assumes that we'll always be running in a single thread (true for Wasm environments)
unsafe impl Send for WasmStorage {}
unsafe impl Sync for WasmStorage {}

#[async_trait]
impl breez_sdk_spark::Storage for WasmStorage {
    async fn get_cached_item(&self, key: String) -> Result<Option<String>, StorageError> {
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

    async fn set_cached_item(&self, key: String, value: String) -> Result<(), StorageError> {
        let promise = self
            .storage
            .set_cached_item(key, value)
            .map_err(js_error_to_storage_error)?;
        let future = JsFuture::from(promise);
        future.await.map_err(js_error_to_storage_error)?;
        Ok(())
    }

    async fn delete_cached_item(&self, key: String) -> Result<(), StorageError> {
        let promise = self
            .storage
            .delete_cached_item(key)
            .map_err(js_error_to_storage_error)?;
        let future = JsFuture::from(promise);
        future.await.map_err(js_error_to_storage_error)?;
        Ok(())
    }

    async fn list_payments(
        &self,
        request: breez_sdk_spark::ListPaymentsRequest,
    ) -> Result<Vec<breez_sdk_spark::Payment>, StorageError> {
        let promise = self
            .storage
            .list_payments(request.into())
            .map_err(js_error_to_storage_error)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_storage_error)?;

        let payments: Vec<Payment> = serde_wasm_bindgen::from_value(result)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        Ok(payments.into_iter().map(|p| p.into()).collect())
    }

    async fn insert_payment(&self, payment: breez_sdk_spark::Payment) -> Result<(), StorageError> {
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
    ) -> Result<(), StorageError> {
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
    ) -> Result<breez_sdk_spark::Payment, StorageError> {
        let promise = self
            .storage
            .get_payment_by_id(id)
            .map_err(js_error_to_storage_error)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_storage_error)?;

        let payment: Payment = serde_wasm_bindgen::from_value(result)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        Ok(payment.into())
    }

    async fn get_payment_by_invoice(
        &self,
        invoice: String,
    ) -> Result<Option<breez_sdk_spark::Payment>, StorageError> {
        let promise = self
            .storage
            .get_payment_by_invoice(invoice)
            .map_err(js_error_to_storage_error)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_storage_error)?;

        let payment: Option<Payment> = serde_wasm_bindgen::from_value(result)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        Ok(payment.map(|p| p.into()))
    }

    async fn add_deposit(
        &self,
        txid: String,
        vout: u32,
        amount_sats: u64,
    ) -> Result<(), StorageError> {
        let promise = self
            .storage
            .add_deposit(txid, vout, amount_sats)
            .map_err(js_error_to_storage_error)?;
        let future = JsFuture::from(promise);
        future.await.map_err(js_error_to_storage_error)?;
        Ok(())
    }

    async fn delete_deposit(&self, txid: String, vout: u32) -> Result<(), StorageError> {
        let promise = self
            .storage
            .delete_deposit(txid, vout)
            .map_err(js_error_to_storage_error)?;
        let future = JsFuture::from(promise);
        future.await.map_err(js_error_to_storage_error)?;
        Ok(())
    }

    async fn list_deposits(&self) -> Result<Vec<breez_sdk_spark::DepositInfo>, StorageError> {
        let promise = self
            .storage
            .list_deposits()
            .map_err(js_error_to_storage_error)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_storage_error)?;

        let deposits: Vec<DepositInfo> = serde_wasm_bindgen::from_value(result)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        Ok(deposits.into_iter().map(|d| d.into()).collect())
    }

    async fn update_deposit(
        &self,
        txid: String,
        vout: u32,
        payload: breez_sdk_spark::UpdateDepositPayload,
    ) -> Result<(), StorageError> {
        let promise = self
            .storage
            .update_deposit(txid, vout, payload.into())
            .map_err(js_error_to_storage_error)?;
        let future = JsFuture::from(promise);
        future.await.map_err(js_error_to_storage_error)?;
        Ok(())
    }
}

#[async_trait]
impl breez_sdk_spark::sync_storage::SyncStorage for WasmStorage {
    async fn add_outgoing_change(
        &self,
        record: breez_sdk_spark::sync_storage::UnversionedRecordChange,
    ) -> Result<u64, SyncStorageError> {
        let promise = self
            .storage
            .sync_add_outgoing_change(record.into())
            .map_err(js_error_to_sync_storage_error)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_sync_storage_error)?;

        let revision: u64 = serde_wasm_bindgen::from_value(result)
            .map_err(|e| SyncStorageError::Serialization(e.to_string()))?;
        Ok(revision)
    }

    async fn complete_outgoing_sync(
        &self,
        record: breez_sdk_spark::sync_storage::Record,
    ) -> Result<(), SyncStorageError> {
        let promise = self
            .storage
            .sync_complete_outgoing_sync(record.into())
            .map_err(js_error_to_sync_storage_error)?;
        let future = JsFuture::from(promise);
        future.await.map_err(js_error_to_sync_storage_error)?;
        Ok(())
    }

    async fn get_pending_outgoing_changes(
        &self,
        limit: u32,
    ) -> Result<Vec<breez_sdk_spark::sync_storage::OutgoingChange>, SyncStorageError> {
        let promise = self
            .storage
            .sync_get_pending_outgoing_changes(limit)
            .map_err(js_error_to_sync_storage_error)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_sync_storage_error)?;

        let changes: Vec<OutgoingChange> = serde_wasm_bindgen::from_value(result)
            .map_err(|e| SyncStorageError::Serialization(e.to_string()))?;
        Ok(changes.into_iter().map(|c| c.into()).collect())
    }

    async fn get_last_revision(&self) -> Result<u64, SyncStorageError> {
        let promise = self
            .storage
            .sync_get_last_revision()
            .map_err(js_error_to_sync_storage_error)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_sync_storage_error)?;

        let revision: u64 = serde_wasm_bindgen::from_value(result)
            .map_err(|e| SyncStorageError::Serialization(e.to_string()))?;
        Ok(revision)
    }

    async fn insert_incoming_records(
        &self,
        records: Vec<breez_sdk_spark::sync_storage::Record>,
    ) -> Result<(), SyncStorageError> {
        let records: Vec<Record> = records.into_iter().map(|r| r.into()).collect();
        let promise = self
            .storage
            .sync_insert_incoming_records(records)
            .map_err(js_error_to_sync_storage_error)?;
        let future = JsFuture::from(promise);
        future.await.map_err(js_error_to_sync_storage_error)?;
        Ok(())
    }

    async fn delete_incoming_record(
        &self,
        record: breez_sdk_spark::sync_storage::Record,
    ) -> Result<(), SyncStorageError> {
        let promise = self
            .storage
            .sync_delete_incoming_record(record.into())
            .map_err(js_error_to_sync_storage_error)?;
        let future = JsFuture::from(promise);
        future.await.map_err(js_error_to_sync_storage_error)?;
        Ok(())
    }

    async fn rebase_pending_outgoing_records(&self, revision: u64) -> Result<(), SyncStorageError> {
        let promise = self
            .storage
            .sync_rebase_pending_outgoing_records(revision)
            .map_err(js_error_to_sync_storage_error)?;
        let future = JsFuture::from(promise);
        future.await.map_err(js_error_to_sync_storage_error)?;
        Ok(())
    }

    async fn get_incoming_records(
        &self,
        limit: u32,
    ) -> Result<Vec<breez_sdk_spark::sync_storage::IncomingChange>, SyncStorageError> {
        let promise = self
            .storage
            .sync_get_incoming_records(limit)
            .map_err(js_error_to_sync_storage_error)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_sync_storage_error)?;

        let records: Vec<IncomingChange> = serde_wasm_bindgen::from_value(result)
            .map_err(|e| SyncStorageError::Serialization(e.to_string()))?;
        Ok(records.into_iter().map(|r| r.into()).collect())
    }

    async fn get_latest_outgoing_change(
        &self,
    ) -> Result<Option<breez_sdk_spark::sync_storage::OutgoingChange>, SyncStorageError> {
        let promise = self
            .storage
            .sync_get_latest_outgoing_change()
            .map_err(js_error_to_sync_storage_error)?;
        let future = JsFuture::from(promise);
        let result = future.await.map_err(js_error_to_sync_storage_error)?;

        if result.is_null() || result.is_undefined() {
            return Ok(None);
        }

        let change_set: OutgoingChange = serde_wasm_bindgen::from_value(result)
            .map_err(|e| SyncStorageError::Serialization(e.to_string()))?;
        Ok(Some(change_set.into()))
    }

    async fn update_record_from_incoming(
        &self,
        record: breez_sdk_spark::sync_storage::Record,
    ) -> Result<(), SyncStorageError> {
        let promise = self
            .storage
            .sync_update_record_from_incoming(record.into())
            .map_err(js_error_to_sync_storage_error)?;
        let future = JsFuture::from(promise);
        future.await.map_err(js_error_to_sync_storage_error)?;
        Ok(())
    }
}

#[wasm_bindgen(typescript_custom_section)]
const STORAGE_INTERFACE: &'static str = r#"export interface Storage {
    getCachedItem: (key: string) => Promise<string | null>;
    setCachedItem: (key: string, value: string) => Promise<void>;
    deleteCachedItem: (key: string) => Promise<void>;
    listPayments: (request: ListPaymentsRequest) => Promise<Payment[]>;
    insertPayment: (payment: Payment) => Promise<void>;
    setPaymentMetadata: (paymentId: string, metadata: PaymentMetadata) => Promise<void>;
    getPaymentById: (id: string) => Promise<Payment>;
    getPaymentByInvoice: (invoice: string) => Promise<Payment>;
    addDeposit: (txid: string, vout: number, amount_sats: number) => Promise<void>;
    deleteDeposit: (txid: string, vout: number) => Promise<void>;
    listDeposits: () => Promise<DepositInfo[]>;
    updateDeposit: (txid: string, vout: number, payload: UpdateDepositPayload) => Promise<void>;
    sync_add_outgoing_change: (record: UnversionedRecordChange) => Promise<number>;
    sync_complete_outgoing_sync: (record: Record) => Promise<void>;
    sync_get_pending_outgoing_changes: (limit: number) => Promise<OutgoingChange[]>;
    sync_get_last_revision: () => Promise<number>;
    sync_insert_incoming_records: (records: Record[]) => Promise<void>;
    sync_delete_incoming_record: (record: Record) => Promise<void>;
    sync_rebase_pending_outgoing_records: (revision: number) => Promise<void>;
    sync_get_incoming_records: (limit: number) => Promise<IncomingChange[]>;
    sync_get_latest_outgoing_change: () => Promise<OutgoingChange | null>;
    sync_update_record_from_incoming: (record: Record) => Promise<void>;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "Storage")]
    pub type Storage;

    #[wasm_bindgen(structural, method, js_name = getCachedItem, catch)]
    pub fn get_cached_item(this: &Storage, key: String) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = setCachedItem, catch)]
    pub fn set_cached_item(this: &Storage, key: String, value: String) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = deleteCachedItem, catch)]
    pub fn delete_cached_item(this: &Storage, key: String) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = listPayments, catch)]
    pub fn list_payments(this: &Storage, request: ListPaymentsRequest) -> Result<Promise, JsValue>;

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

    #[wasm_bindgen(structural, method, js_name = getPaymentByInvoice, catch)]
    pub fn get_payment_by_invoice(this: &Storage, invoice: String) -> Result<Promise, JsValue>;

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

    #[wasm_bindgen(structural, method, js_name = sync_add_outgoing_change, catch)]
    pub fn sync_add_outgoing_change(
        this: &Storage,
        record: UnversionedRecordChange,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = sync_complete_outgoing_sync, catch)]
    pub fn sync_complete_outgoing_sync(this: &Storage, record: Record) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = sync_get_pending_outgoing_changes, catch)]
    pub fn sync_get_pending_outgoing_changes(
        this: &Storage,
        limit: u32,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = sync_get_last_revision, catch)]
    pub fn sync_get_last_revision(this: &Storage) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = sync_insert_incoming_records, catch)]
    pub fn sync_insert_incoming_records(
        this: &Storage,
        records: Vec<Record>,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = sync_delete_incoming_record, catch)]
    pub fn sync_delete_incoming_record(this: &Storage, record: Record) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = sync_rebase_pending_outgoing_records, catch)]
    pub fn sync_rebase_pending_outgoing_records(
        this: &Storage,
        revision: u64,
    ) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = sync_get_incoming_records, catch)]
    pub fn sync_get_incoming_records(this: &Storage, limit: u32) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = sync_get_latest_outgoing_change, catch)]
    pub fn sync_get_latest_outgoing_change(this: &Storage) -> Result<Promise, JsValue>;

    #[wasm_bindgen(structural, method, js_name = sync_update_record_from_incoming, catch)]
    pub fn sync_update_record_from_incoming(
        this: &Storage,
        record: Record,
    ) -> Result<Promise, JsValue>;
}
