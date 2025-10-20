use wasm_bindgen::prelude::*;

use crate::models::ReceiverTokenOutput;

pub struct WasmPaymentObserver {
    pub payment_observer: PaymentObserver,
}

// This assumes that we'll always be running in a single thread (true for Wasm environments)
unsafe impl Send for WasmPaymentObserver {}
unsafe impl Sync for WasmPaymentObserver {}

#[macros::async_trait]
impl breez_sdk_spark::PaymentObserver for WasmPaymentObserver {
    async fn before_send_bitcoin(
        &self,
        payment_id: String,
        withdrawal_address: String,
        amount_sats: u64,
    ) -> Result<(), breez_sdk_spark::PaymentObserverError> {
        self.payment_observer
            .on_send_bitcoin(payment_id, withdrawal_address, amount_sats);
        Ok(())
    }

    async fn before_send_lightning(
        &self,
        payment_id: String,
        invoice: String,
        amount_sats: u64,
    ) -> Result<(), breez_sdk_spark::PaymentObserverError> {
        self.payment_observer
            .on_send_lightning(payment_id, invoice, amount_sats);
        Ok(())
    }

    async fn before_send_spark(
        &self,
        payment_id: String,
        receiver_public_key: String,
        amount_sats: u64,
    ) -> Result<(), breez_sdk_spark::PaymentObserverError> {
        self.payment_observer
            .on_send_spark(payment_id, receiver_public_key, amount_sats);
        Ok(())
    }

    async fn before_send_token(
        &self,
        tx_id: String,
        token_id: String,
        receiver_outputs: Vec<breez_sdk_spark::ReceiverTokenOutput>,
    ) -> Result<(), breez_sdk_spark::PaymentObserverError> {
        self.payment_observer.on_send_token(
            tx_id,
            token_id,
            receiver_outputs.into_iter().map(Into::into).collect(),
        );
        Ok(())
    }
}

#[wasm_bindgen(typescript_custom_section)]
const EVENT_INTERFACE: &'static str = r#"export interface PaymentObserver {
    onSendBitcoin: (paymentId: string, withdrawalAddress: string, amountSats: number) => void;
    onSendLightning: (paymentId: string, invoice: string, amountSats: number) => void;
    onSendSpark: (paymentId: string, receiverPublicKey: string, amountSats: number) => void;
    onSendToken: (txId: string, tokenId: string, receiverOutputs: ReceiverTokenOutput[]) => void;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "PaymentObserver")]
    pub type PaymentObserver;

    #[wasm_bindgen(structural, method, js_name = onSendBitcoin)]
    pub fn on_send_bitcoin(
        this: &PaymentObserver,
        payment_id: String,
        withdrawal_address: String,
        amount_sats: u64,
    );
    #[wasm_bindgen(structural, method, js_name = onSendLightning)]
    pub fn on_send_lightning(
        this: &PaymentObserver,
        payment_id: String,
        invoice: String,
        amount_sats: u64,
    );
    #[wasm_bindgen(structural, method, js_name = onSendSpark)]
    pub fn on_send_spark(
        this: &PaymentObserver,
        payment_id: String,
        receiver_public_key: String,
        amount_sats: u64,
    );
    #[wasm_bindgen(structural, method, js_name = onSendToken)]
    pub fn on_send_token(
        this: &PaymentObserver,
        tx_id: String,
        token_id: String,
        receiver_outputs: Vec<ReceiverTokenOutput>,
    );
}
