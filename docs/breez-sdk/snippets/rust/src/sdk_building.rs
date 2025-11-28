use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use breez_sdk_spark::*;
use log::info;

pub(crate) async fn init_sdk_advanced() -> Result<BreezSdk> {
    // ANCHOR: init-sdk-advanced
    // Construct the seed using mnemonic words or entropy bytes
    let mnemonic = "<mnemonic words>".to_string();
    let seed = Seed::Mnemonic {
        mnemonic,
        passphrase: None,
    };

    // Create the default config
    let mut config = default_config(Network::Mainnet);
    config.api_key = Some("<breez api key>".to_string());

    // Build the SDK using the config, seed and default storage
    let builder = SdkBuilder::new(config, seed).with_default_storage("./.data".to_string());
    // You can also pass your custom implementations:
    // let builder = builder.with_storage(<your storage implementation>)
    // let builder = builder.with_real_time_sync_storage(<your real-time sync storage implementation>)
    // let builder = builder.with_chain_service(<your chain service implementation>)
    // let builder = builder.with_rest_client(<your rest client implementation>)
    // let builder = builder.with_key_set(<your key set type>, <use address index>, <account number>)
    // let builder = builder.with_payment_observer(<your payment observer implementation>);
    let sdk = builder.build().await?;

    // ANCHOR_END: init-sdk-advanced
    Ok(sdk)
}

pub(crate) fn with_rest_chain_service(builder: SdkBuilder) -> SdkBuilder {
    // ANCHOR: with-rest-chain-service
    let url = "<your REST chain service URL>".to_string();
    let chain_api_type = ChainApiType::MempoolSpace;
    let optional_credentials = Credentials {
        username: "<username>".to_string(),
        password: "<password>".to_string(),
    };
    builder.with_rest_chain_service(
        url,
        chain_api_type,
        Some(optional_credentials),
    )
    // ANCHOR_END: with-rest-chain-service
}

pub(crate) fn with_key_set(builder: SdkBuilder) -> SdkBuilder {
    // ANCHOR: with-key-set
    let key_set_type = KeySetType::Default;
    let use_address_index = false;
    let optional_account_number = 21;
    builder.with_key_set(
        key_set_type,
        use_address_index,
        Some(optional_account_number),
    )
    // ANCHOR_END: with-key-set
}

// ANCHOR: with-payment-observer
pub(crate) struct ExamplePaymentObserver {}

#[async_trait]
impl PaymentObserver for ExamplePaymentObserver {
    async fn before_send(
        &self,
        payments: Vec<ProvisionalPayment>,
    ) -> Result<(), PaymentObserverError> {
        for payment in payments {
            info!(
                "About to send payment: {:?} of amount {:?}",
                payment.payment_id, payment.amount
            );
        }
        Ok(())
    }
}

pub(crate) fn with_payment_observer(builder: SdkBuilder) -> SdkBuilder {
    let observer = ExamplePaymentObserver {};
    builder.with_payment_observer(Arc::new(observer))
}
// ANCHOR_END: with-payment-observer
