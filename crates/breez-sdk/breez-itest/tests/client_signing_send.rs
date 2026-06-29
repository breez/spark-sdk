use anyhow::Result;
use breez_sdk_itest::{
    SdkInstance, build_sdk_with_external_signer, build_sdk_with_external_signer_and_config,
    ensure_funded, wait_for_payment_succeeded_event,
};
use breez_sdk_spark::{
    BuildTransferPackageOptions, BuildUnsignedTransferPackageRequest, CreateIssuerTokenRequest,
    GetInfoRequest, LeafOptimizationConfig, MintIssuerTokenRequest, Network,
    OnchainConfirmationSpeed, PaymentRequest, PaymentStatus, PaymentType,
    PrepareSendPaymentRequest, PublishSignedTransferPackageRequest,
    PublishSignedTransferPackageResponse, ReceivePaymentMethod, ReceivePaymentRequest,
    SignedTransferPackage, SyncWalletRequest, TransferSignature, UnsignedTransferPackage,
    default_config, default_external_signers,
};
use rand::RngCore;
use tracing::info;

fn random_mnemonic() -> Result<String> {
    let mut entropy = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut entropy);
    Ok(bip39::Mnemonic::from_entropy(&entropy)?.to_string())
}

async fn build_alice_manual_opt(mnemonic: String) -> Result<SdkInstance> {
    let dir = tempfile::Builder::new()
        .prefix("breez-sdk-alice-client-signing")
        .tempdir()?;
    let path = dir.path().to_string_lossy().to_string();

    let mut cfg = default_config(Network::Regtest);
    cfg.leaf_optimization_config = LeafOptimizationConfig {
        auto_enabled: false,
        multiplicity: 15,
    };
    build_sdk_with_external_signer_and_config(path, mnemonic, cfg, Some(dir)).await
}

async fn build_alice_default(mnemonic: String) -> Result<SdkInstance> {
    let dir = tempfile::Builder::new()
        .prefix("breez-sdk-alice-client-signing")
        .tempdir()?;
    let path = dir.path().to_string_lossy().to_string();
    build_sdk_with_external_signer(path, mnemonic, Some(dir)).await
}

async fn build_bob() -> Result<SdkInstance> {
    let dir = tempfile::Builder::new()
        .prefix("breez-sdk-bob-client-signing")
        .tempdir()?;
    let path = dir.path().to_string_lossy().to_string();
    build_sdk_with_external_signer(path, random_mnemonic()?, Some(dir)).await
}

#[test_log::test(tokio::test)]
async fn test_client_signing_send_with_denomination_swap() -> Result<()> {
    info!("=== Starting test_client_signing_send_with_denomination_swap ===");

    let fund_sats: u64 = 50_000;
    let send_sats: u128 = 12_345;

    let alice_mnemonic = random_mnemonic()?;
    let mut alice = build_alice_manual_opt(alice_mnemonic.clone()).await?;
    let mut bob = build_bob().await?;

    let client_signer =
        default_external_signers(alice_mnemonic, None, Network::Regtest, None)?.spark_signer;

    ensure_funded(&mut alice, fund_sats).await?;

    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    let bob_initial_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    let mut saw_swap = false;
    let mut iterations = 0;

    loop {
        iterations += 1;
        assert!(
            iterations <= 10,
            "client-signing swap loop did not converge within 10 iterations"
        );

        let prep = alice
            .sdk
            .prepare_send_payment(PrepareSendPaymentRequest {
                payment_request: PaymentRequest::Input {
                    input: bob_spark_address.clone(),
                },
                amount: Some(send_sats),
                token_identifier: None,
                conversion_options: None,
                fee_policy: None,
            })
            .await?;

        let unsigned = alice
            .sdk
            .build_unsigned_transfer_package(BuildUnsignedTransferPackageRequest {
                prepare_response: prep.clone(),
                options: None,
            })
            .await?;

        let signature = match &unsigned {
            UnsignedTransferPackage::Transfer { prepare_transfer }
            | UnsignedTransferPackage::Swap {
                prepare_transfer, ..
            } => TransferSignature::Transfer {
                signed: client_signer
                    .prepare_transfer(prepare_transfer.clone())
                    .await?,
            },
            UnsignedTransferPackage::Token { .. } => {
                panic!("unexpected token package for a sats send")
            }
        };
        let signed_package = SignedTransferPackage {
            unsigned,
            signature,
        };

        match alice
            .sdk
            .publish_signed_transfer_package(PublishSignedTransferPackageRequest {
                prepare_response: prep.clone(),
                signed_package,
            })
            .await?
        {
            PublishSignedTransferPackageResponse::SwapCompleted => {
                info!("client-signing iteration {iterations}: swap required");
                saw_swap = true;
                continue;
            }
            PublishSignedTransferPackageResponse::PaymentSent { payment } => {
                info!("client-signing iteration {iterations}: transfer ready, sent");
                assert!(
                    matches!(
                        payment.status,
                        PaymentStatus::Completed | PaymentStatus::Pending
                    ),
                    "Payment should be completed or pending"
                );
                break;
            }
        }
    }

    assert!(
        saw_swap,
        "expected at least one denomination swap iteration"
    );

    let received_payment =
        wait_for_payment_succeeded_event(&mut bob.events, PaymentType::Receive, 60).await?;
    assert_eq!(received_payment.payment_type, PaymentType::Receive);
    assert_eq!(received_payment.amount, send_sats);
    assert_eq!(received_payment.status, PaymentStatus::Completed);

    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_final_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .balance_sats;

    assert_eq!(
        bob_final_balance,
        bob_initial_balance + send_sats as u64,
        "Bob's balance should increase by the sent amount"
    );

    info!("=== Test test_client_signing_send_with_denomination_swap PASSED ===");
    Ok(())
}

#[test_log::test(tokio::test)]
async fn test_client_signing_token_send() -> Result<()> {
    info!("=== Starting test_client_signing_token_send ===");

    let send_amount: u128 = 5;

    let alice_mnemonic = random_mnemonic()?;
    let alice = build_alice_default(alice_mnemonic.clone()).await?;
    let bob = build_bob().await?;

    let client_signer =
        default_external_signers(alice_mnemonic, None, Network::Regtest, None)?.spark_signer;

    let issuer = alice.sdk.get_token_issuer();
    let token_metadata = issuer
        .create_issuer_token(CreateIssuerTokenRequest {
            name: "client-signing token".to_string(),
            ticker: "CST".to_string(),
            decimals: 2,
            is_freezable: false,
            max_supply: 1_000_000,
        })
        .await?;
    issuer
        .mint_issuer_token(MintIssuerTokenRequest { amount: 1_000_000 })
        .await?;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    alice.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let token_id = token_metadata.identifier.clone();

    let bob_spark_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::SparkAddress,
        })
        .await?
        .payment_request;

    let prep = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: bob_spark_address.clone(),
            },
            amount: Some(send_amount),
            token_identifier: Some(token_id.clone()),
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    let unsigned = alice
        .sdk
        .build_unsigned_transfer_package(BuildUnsignedTransferPackageRequest {
            prepare_response: prep.clone(),
            options: None,
        })
        .await?;

    let UnsignedTransferPackage::Token {
        prepare_token_transaction,
        ..
    } = &unsigned
    else {
        panic!("expected a Token unsigned package for a token send");
    };
    let signed = client_signer
        .prepare_token_transaction(prepare_token_transaction.clone())
        .await?;
    let signature = TransferSignature::Token { signed };

    let PublishSignedTransferPackageResponse::PaymentSent { payment } = alice
        .sdk
        .publish_signed_transfer_package(PublishSignedTransferPackageRequest {
            prepare_response: prep,
            signed_package: SignedTransferPackage {
                unsigned,
                signature,
            },
        })
        .await?
    else {
        panic!("expected a sent payment for a token send");
    };
    assert!(
        matches!(
            payment.status,
            PaymentStatus::Completed | PaymentStatus::Pending
        ),
        "Payment should be completed or pending"
    );

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    bob.sdk.sync_wallet(SyncWalletRequest {}).await?;
    let bob_token_balance = bob
        .sdk
        .get_info(GetInfoRequest {
            ensure_synced: Some(false),
        })
        .await?
        .token_balances
        .get(&token_id)
        .map(|b| b.balance)
        .unwrap_or(0);
    assert_eq!(
        bob_token_balance, send_amount,
        "Bob should have received the token amount"
    );

    info!("=== Test test_client_signing_token_send PASSED ===");
    Ok(())
}

#[test_log::test(tokio::test)]
async fn test_client_signing_coop_exit() -> Result<()> {
    info!("=== Starting test_client_signing_coop_exit ===");

    let fund_sats: u64 = 50_000;
    let withdraw_sats: u128 = 15_000;

    let alice_mnemonic = random_mnemonic()?;
    let mut alice = build_alice_default(alice_mnemonic.clone()).await?;
    let bob = build_bob().await?;

    let client_signer =
        default_external_signers(alice_mnemonic, None, Network::Regtest, None)?.spark_signer;

    ensure_funded(&mut alice, fund_sats).await?;

    let withdrawal_address = bob
        .sdk
        .receive_payment(ReceivePaymentRequest {
            payment_method: ReceivePaymentMethod::BitcoinAddress { new_address: None },
        })
        .await?
        .payment_request;
    info!("Withdrawal address: {withdrawal_address}");

    let prep = alice
        .sdk
        .prepare_send_payment(PrepareSendPaymentRequest {
            payment_request: PaymentRequest::Input {
                input: withdrawal_address,
            },
            amount: Some(withdraw_sats),
            token_identifier: None,
            conversion_options: None,
            fee_policy: None,
        })
        .await?;

    let mut iterations = 0;
    loop {
        iterations += 1;
        assert!(
            iterations <= 10,
            "client-signing coop-exit swap loop did not converge within 10 iterations"
        );

        let unsigned = alice
            .sdk
            .build_unsigned_transfer_package(BuildUnsignedTransferPackageRequest {
                prepare_response: prep.clone(),
                options: Some(BuildTransferPackageOptions::BitcoinAddress {
                    confirmation_speed: OnchainConfirmationSpeed::Fast,
                }),
            })
            .await?;

        let signature = match &unsigned {
            UnsignedTransferPackage::Transfer { prepare_transfer }
            | UnsignedTransferPackage::Swap {
                prepare_transfer, ..
            } => TransferSignature::Transfer {
                signed: client_signer
                    .prepare_transfer(prepare_transfer.clone())
                    .await?,
            },
            UnsignedTransferPackage::Token { .. } => {
                panic!("unexpected token package for a coop-exit")
            }
        };
        let signed_package = SignedTransferPackage {
            unsigned,
            signature,
        };

        match alice
            .sdk
            .publish_signed_transfer_package(PublishSignedTransferPackageRequest {
                prepare_response: prep.clone(),
                signed_package,
            })
            .await?
        {
            PublishSignedTransferPackageResponse::SwapCompleted => {
                info!("coop-exit iteration {iterations}: swap required");
                continue;
            }
            PublishSignedTransferPackageResponse::PaymentSent { payment } => {
                info!("coop-exit iteration {iterations}: transfer ready, sent");
                assert_eq!(payment.payment_type, PaymentType::Send);
                assert!(
                    matches!(
                        payment.status,
                        PaymentStatus::Completed | PaymentStatus::Pending
                    ),
                    "Coop-exit payment should be completed or pending"
                );
                break;
            }
        }
    }

    info!("=== Test test_client_signing_coop_exit PASSED ===");
    Ok(())
}
