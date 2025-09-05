use breez_sdk_spark::{
    DepositInfo, Payment, PaymentMetadata, Storage, StorageError, UpdateDepositPayload,
};

pub struct NopStorage;

#[async_trait::async_trait]
impl Storage for NopStorage {
    async fn get_cached_item(&self, _key: String) -> Result<Option<String>, StorageError> {
        Ok(None)
    }

    async fn set_cached_item(&self, _key: String, _value: String) -> Result<(), StorageError> {
        Ok(())
    }

    async fn list_payments(
        &self,
        _offset: Option<u32>,
        _limit: Option<u32>,
    ) -> Result<Vec<Payment>, StorageError> {
        Ok(Vec::new())
    }

    async fn insert_payment(&self, _payment: Payment) -> Result<(), StorageError> {
        Ok(())
    }

    async fn set_payment_metadata(
        &self,
        _payment_id: String,
        _metadata: PaymentMetadata,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    async fn get_payment_by_id(&self, _id: String) -> Result<Payment, StorageError> {
        Err(StorageError::Implementation("not implemented".to_string()))
    }

    async fn add_deposit(
        &self,
        _txid: String,
        _vout: u32,
        _amount_sats: u64,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    async fn delete_deposit(&self, _txid: String, _vout: u32) -> Result<(), StorageError> {
        Ok(())
    }

    async fn list_deposits(&self) -> Result<Vec<DepositInfo>, StorageError> {
        Ok(Vec::new())
    }

    async fn update_deposit(
        &self,
        _txid: String,
        _vout: u32,
        _payload: UpdateDepositPayload,
    ) -> Result<(), StorageError> {
        Ok(())
    }
}
