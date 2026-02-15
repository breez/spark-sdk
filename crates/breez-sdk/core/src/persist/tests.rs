use std::collections::HashMap;

use chrono::Utc;

use crate::{
    DepositClaimError, ListPaymentsRequest, LnurlWithdrawInfo, Payment, PaymentDetails,
    PaymentMetadata, PaymentMethod, PaymentStatus, PaymentType, SparkHtlcDetails, SparkHtlcStatus,
    Storage, TokenMetadata, TokenTransactionType, UpdateDepositPayload,
    persist::ObjectCacheRepository,
    sync_storage::{Record, RecordId, UnversionedRecordChange},
};

#[allow(clippy::too_many_lines)]
pub async fn test_sync_storage(storage: Box<dyn Storage>) {
    use std::collections::HashMap;

    // Test 1: Initial state - get_last_revision should return 0
    let last_revision = storage.get_last_revision().await.unwrap();
    assert_eq!(last_revision, 0, "Initial last revision should be 0");

    // Test 2: No pending outgoing changes initially
    let pending = storage.get_pending_outgoing_changes(10).await.unwrap();
    assert_eq!(pending.len(), 0, "Should have no pending outgoing changes");

    // Test 3: No incoming records initially
    let incoming = storage.get_incoming_records(10).await.unwrap();
    assert_eq!(incoming.len(), 0, "Should have no incoming records");

    // Test 4: No latest outgoing change initially
    let latest = storage.get_latest_outgoing_change().await.unwrap();
    assert!(latest.is_none(), "Should have no latest outgoing change");

    // Test 5: Add outgoing change (create new record)
    let mut updated_fields = HashMap::new();
    updated_fields.insert("name".to_string(), "\"Alice\"".to_string());
    updated_fields.insert("age".to_string(), "30".to_string());

    let change1 = UnversionedRecordChange {
        id: RecordId::new("user".to_string(), "user1".to_string()),
        schema_version: "1.0.0".to_string(),
        updated_fields: updated_fields.clone(),
    };

    let revision1 = storage.add_outgoing_change(change1).await.unwrap();
    assert!(revision1 > 0, "First revision should be greater than 0");

    // Test 6: Check pending outgoing changes
    let pending = storage.get_pending_outgoing_changes(10).await.unwrap();
    assert_eq!(pending.len(), 1, "Should have 1 pending outgoing change");
    assert_eq!(pending[0].change.id.r#type, "user");
    assert_eq!(pending[0].change.id.data_id, "user1");
    assert_eq!(pending[0].change.local_revision, revision1);
    assert_eq!(pending[0].change.schema_version, "1.0.0");
    assert!(
        pending[0].parent.is_none(),
        "First change should have no parent"
    );

    // Test 7: Get latest outgoing change
    let latest = storage.get_latest_outgoing_change().await.unwrap();
    assert!(latest.is_some());
    let latest = latest.unwrap();
    assert_eq!(latest.change.id.r#type, "user");
    assert_eq!(latest.change.local_revision, revision1);

    // Test 8: Complete outgoing sync (moves to sync_state)
    let mut complete_data = HashMap::new();
    complete_data.insert("name".to_string(), "\"Alice\"".to_string());
    complete_data.insert("age".to_string(), "30".to_string());

    let completed_record = Record {
        id: RecordId::new("user".to_string(), "user1".to_string()),
        revision: revision1,
        schema_version: "1.0.0".to_string(),
        data: complete_data,
    };

    storage
        .complete_outgoing_sync(completed_record.clone(), revision1)
        .await
        .unwrap();

    // Test 9: Pending changes should now be empty
    let pending = storage.get_pending_outgoing_changes(10).await.unwrap();
    assert_eq!(
        pending.len(),
        0,
        "Should have no pending changes after completion"
    );

    // Test 10: Last revision should be updated
    let last_revision = storage.get_last_revision().await.unwrap();
    assert_eq!(
        last_revision, revision1,
        "Last revision should match completed revision"
    );

    // Test 11: Add another outgoing change (update existing record)
    let mut updated_fields2 = HashMap::new();
    updated_fields2.insert("age".to_string(), "31".to_string());

    let change2 = UnversionedRecordChange {
        id: RecordId::new("user".to_string(), "user1".to_string()),
        schema_version: "1.0.0".to_string(),
        updated_fields: updated_fields2,
    };

    let revision2 = storage.add_outgoing_change(change2).await.unwrap();
    assert!(revision2 > 0, "Second local queue id should be positive");

    // Test 12: Check pending changes now includes parent
    let pending = storage.get_pending_outgoing_changes(10).await.unwrap();
    assert_eq!(pending.len(), 1, "Should have 1 pending change");
    assert!(
        pending[0].parent.is_some(),
        "Update should have parent record"
    );
    let parent = pending[0].parent.as_ref().unwrap();
    assert_eq!(parent.revision, revision1);
    assert_eq!(parent.id.r#type, "user");

    // Test 13: Insert incoming records
    let mut incoming_data1 = HashMap::new();
    incoming_data1.insert("title".to_string(), "\"Post 1\"".to_string());
    incoming_data1.insert("content".to_string(), "\"Hello World\"".to_string());

    let incoming_record1 = Record {
        id: RecordId::new("post".to_string(), "post1".to_string()),
        revision: 100,
        schema_version: "1.0.0".to_string(),
        data: incoming_data1,
    };

    let mut incoming_data2 = HashMap::new();
    incoming_data2.insert("title".to_string(), "\"Post 2\"".to_string());

    let incoming_record2 = Record {
        id: RecordId::new("post".to_string(), "post2".to_string()),
        revision: 101,
        schema_version: "1.0.0".to_string(),
        data: incoming_data2,
    };

    storage
        .insert_incoming_records(vec![incoming_record1.clone(), incoming_record2.clone()])
        .await
        .unwrap();

    // Test 14: Get incoming records
    let incoming = storage.get_incoming_records(10).await.unwrap();
    assert_eq!(incoming.len(), 2, "Should have 2 incoming records");
    assert_eq!(incoming[0].new_state.id.r#type, "post");
    assert_eq!(incoming[0].new_state.revision, 100);
    assert!(
        incoming[0].old_state.is_none(),
        "New incoming record should have no old state"
    );

    // Test 15: Update record from incoming (moves to sync_state)
    storage
        .update_record_from_incoming(incoming_record1.clone())
        .await
        .unwrap();

    // Test 16: Delete incoming record
    storage
        .delete_incoming_record(incoming_record1.clone())
        .await
        .unwrap();

    // Test 17: Check incoming records after deletion
    let incoming = storage.get_incoming_records(10).await.unwrap();
    assert_eq!(incoming.len(), 1, "Should have 1 incoming record remaining");
    assert_eq!(incoming[0].new_state.id.data_id, "post2");

    // Test 18: Insert incoming record that updates existing state
    let mut updated_incoming_data = HashMap::new();
    updated_incoming_data.insert("title".to_string(), "\"Post 1 Updated\"".to_string());
    updated_incoming_data.insert("content".to_string(), "\"Updated content\"".to_string());

    let updated_incoming_record = Record {
        id: RecordId::new("post".to_string(), "post1".to_string()),
        revision: 102,
        schema_version: "1.0.0".to_string(),
        data: updated_incoming_data,
    };

    storage
        .insert_incoming_records(vec![updated_incoming_record.clone()])
        .await
        .unwrap();

    // Test 19: Get incoming records with old_state
    let incoming = storage.get_incoming_records(10).await.unwrap();
    let post1_update = incoming.iter().find(|r| r.new_state.id.data_id == "post1");
    assert!(post1_update.is_some(), "Should find post1 update");
    let post1_update = post1_update.unwrap();
    assert!(
        post1_update.old_state.is_some(),
        "Update should have old state"
    );
    assert_eq!(
        post1_update.old_state.as_ref().unwrap().revision,
        100,
        "Old state should be original revision"
    );

    // Test 20: Update sync state with a higher incoming revision.
    let cursor_bump_record = Record {
        id: RecordId::new("cursor".to_string(), "bump".to_string()),
        revision: 150,
        schema_version: "1.0.0".to_string(),
        data: HashMap::new(),
    };
    storage
        .update_record_from_incoming(cursor_bump_record)
        .await
        .unwrap();

    // Test 21: Pending outgoing local queue ids should remain unchanged, while the
    // committed sync cursor advances when incoming state is applied.
    let pending = storage.get_pending_outgoing_changes(10).await.unwrap();
    assert!(
        pending[0].change.local_revision == revision2,
        "Pending outgoing local queue id should remain unchanged"
    );
    let last_revision = storage.get_last_revision().await.unwrap();
    assert_eq!(
        last_revision, 150,
        "Committed sync cursor should be advanced"
    );

    // Test 22: Test limit on pending outgoing changes
    // Add multiple changes
    for i in 0..5 {
        let mut fields = HashMap::new();
        fields.insert("value".to_string(), format!("\"{i}\""));

        let change = UnversionedRecordChange {
            id: RecordId::new("test".to_string(), format!("test{i}")),
            schema_version: "1.0.0".to_string(),
            updated_fields: fields,
        };
        storage.add_outgoing_change(change).await.unwrap();
    }

    let pending_limited = storage.get_pending_outgoing_changes(3).await.unwrap();
    assert_eq!(
        pending_limited.len(),
        3,
        "Should respect limit on pending changes"
    );

    // Test 23: Test limit on incoming records
    let incoming_limited = storage.get_incoming_records(1).await.unwrap();
    assert_eq!(
        incoming_limited.len(),
        1,
        "Should respect limit on incoming records"
    );

    // Test 24: Test ordering - pending outgoing should be ordered by revision ASC
    let all_pending = storage.get_pending_outgoing_changes(100).await.unwrap();
    for i in 1..all_pending.len() {
        assert!(
            all_pending[i].change.local_revision
                >= all_pending[i.saturating_sub(1)].change.local_revision,
            "Pending changes should be ordered by local queue id ascending"
        );
    }

    // Test 25: Test ordering - incoming should be ordered by revision ASC
    let all_incoming = storage.get_incoming_records(100).await.unwrap();
    for i in 1..all_incoming.len() {
        assert!(
            all_incoming[i].new_state.revision
                >= all_incoming[i.saturating_sub(1)].new_state.revision,
            "Incoming records should be ordered by revision ascending"
        );
    }

    // Test 26: Test empty insert_incoming_records
    storage.insert_incoming_records(vec![]).await.unwrap();

    // Test 27: Test different record types
    let mut settings_fields = HashMap::new();
    settings_fields.insert("theme".to_string(), "\"dark\"".to_string());

    let settings_change = UnversionedRecordChange {
        id: RecordId::new("settings".to_string(), "global".to_string()),
        schema_version: "2.0.0".to_string(),
        updated_fields: settings_fields,
    };

    let settings_revision = storage.add_outgoing_change(settings_change).await.unwrap();

    let pending = storage.get_pending_outgoing_changes(100).await.unwrap();
    let settings_pending = pending.iter().find(|p| p.change.id.r#type == "settings");
    assert!(settings_pending.is_some(), "Should find settings change");
    assert_eq!(
        settings_pending.unwrap().change.schema_version,
        "2.0.0",
        "Should preserve schema version"
    );

    // Test 28: Complete multiple types
    let mut complete_settings_data = HashMap::new();
    complete_settings_data.insert("theme".to_string(), "\"dark\"".to_string());

    let completed_settings = Record {
        id: RecordId::new("settings".to_string(), "global".to_string()),
        revision: settings_revision,
        schema_version: "2.0.0".to_string(),
        data: complete_settings_data,
    };

    storage
        .complete_outgoing_sync(completed_settings, settings_revision)
        .await
        .unwrap();

    let last_revision = storage.get_last_revision().await.unwrap();
    assert!(
        last_revision >= settings_revision,
        "Last revision should be at least settings revision"
    );
}

#[allow(clippy::too_many_lines)]
pub async fn test_storage(storage: Box<dyn Storage>) {
    use crate::SetLnurlMetadataItem;
    use crate::models::{LnurlPayInfo, TokenMetadata};

    // Test 1: Spark invoice payment
    let spark_payment = Payment {
        id: "spark_pmt123".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: u128::from(u64::MAX).checked_add(100_000).unwrap(),
        fees: 1_000,
        timestamp: 5_000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: Some(crate::SparkInvoicePaymentDetails {
                description: Some("description".to_string()),
                invoice: "invoice_string".to_string(),
            }),
            htlc_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    // Test 2: Spark HTLC payment
    let spark_htlc_payment = Payment {
        id: "spark_htlc_pmt123".to_string(),
        payment_type: PaymentType::Receive,
        status: PaymentStatus::Completed,
        amount: 20_000,
        fees: 2_000,
        timestamp: 10_000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: Some(SparkHtlcDetails {
                payment_hash: "payment_hash123".to_string(),
                preimage: Some("preimage123".to_string()),
                expiry_time: 15_000,
                status: SparkHtlcStatus::PreimageShared,
            }),
            conversion_info: None,
        }),
        conversion_details: None,
    };

    // Test 3: Transfer token payment with invoice
    let token_metadata = TokenMetadata {
        identifier: "token123".to_string(),
        issuer_public_key: "02abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890ab"
            .to_string(),
        name: "Test Token".to_string(),
        ticker: "TTK".to_string(),
        decimals: 8,
        max_supply: 21_000_000,
        is_freezable: false,
    };
    let token_transfer_payment = Payment {
        id: "token_transfer_pmt456".to_string(),
        payment_type: PaymentType::Receive,
        status: PaymentStatus::Pending,
        amount: 50_000,
        fees: 500,
        timestamp: Utc::now().timestamp().try_into().unwrap(),
        method: PaymentMethod::Token,
        details: Some(PaymentDetails::Token {
            metadata: token_metadata.clone(),
            tx_hash: "tx_hash".to_string(),
            tx_type: TokenTransactionType::Transfer,
            invoice_details: Some(crate::SparkInvoicePaymentDetails {
                description: Some("description_2".to_string()),
                invoice: "invoice_string_2".to_string(),
            }),
            conversion_info: None,
        }),
        conversion_details: None,
    };

    // Test 4: Mint token payment
    let token_mint_payment = Payment {
        id: "token_mint_pmt789".to_string(),
        payment_type: PaymentType::Receive,
        status: PaymentStatus::Completed,
        amount: 100_000,
        fees: 0,
        timestamp: Utc::now().timestamp().try_into().unwrap(),
        method: PaymentMethod::Token,
        details: Some(PaymentDetails::Token {
            metadata: token_metadata.clone(),
            tx_hash: "tx_hash_mint".to_string(),
            tx_type: TokenTransactionType::Mint,
            invoice_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    // Test 5: Burn token payment
    let token_burn_payment = Payment {
        id: "token_burn_pmt012".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 20_000,
        fees: 0,
        timestamp: Utc::now().timestamp().try_into().unwrap(),
        method: PaymentMethod::Token,
        details: Some(PaymentDetails::Token {
            metadata: token_metadata.clone(),
            tx_hash: "tx_hash_burn".to_string(),
            tx_type: TokenTransactionType::Burn,
            invoice_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    // Test 6: Lightning payment with full details
    let pay_metadata = PaymentMetadata {
        lnurl_pay_info: Some(LnurlPayInfo {
            ln_address: Some("test@example.com".to_string()),
            comment: Some("Test comment".to_string()),
            domain: Some("example.com".to_string()),
            metadata: Some("[[\"text/plain\", \"Test metadata\"]]".to_string()),
            processed_success_action: None,
            raw_success_action: None,
        }),
        ..Default::default()
    };

    let lightning_lnurl_pay_payment = Payment {
        id: "lightning_pmt789".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 25_000,
        fees: 250,
        timestamp: Utc::now().timestamp().try_into().unwrap(),
        method: PaymentMethod::Lightning,
        details: Some(PaymentDetails::Lightning {
            description: Some("Test lightning payment".to_string()),
            preimage: Some("abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890ab".to_string()),
            invoice: "lnbc250n1pjqxyz9pp5abc123def456ghi789jkl012mno345pqr678stu901vwx234yz567890abcdefghijklmnopqrstuvwxyz".to_string(),
            payment_hash: "fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321".to_string(),
            destination_pubkey: "03123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef01".to_string(),
            lnurl_pay_info: pay_metadata.lnurl_pay_info.clone(),
            lnurl_withdraw_info: pay_metadata.lnurl_withdraw_info.clone(),
            lnurl_receive_metadata: None,
        }),
        conversion_details: None,
    };

    // Test 7: Lightning payment with full details
    let withdraw_metadata = PaymentMetadata {
        lnurl_withdraw_info: Some(LnurlWithdrawInfo {
            withdraw_url: "http://example.com/withdraw".to_string(),
        }),
        ..Default::default()
    };
    let lightning_lnurl_withdraw_payment = Payment {
        id: "lightning_pmtabc".to_string(),
        payment_type: PaymentType::Receive,
        status: PaymentStatus::Completed,
        amount: 75_000,
        fees: 750,
        timestamp: Utc::now().timestamp().try_into().unwrap(),
        method: PaymentMethod::Lightning,
        details: Some(PaymentDetails::Lightning {
            description: Some("Test lightning payment".to_string()),
            preimage: Some("abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890ab".to_string()),
            invoice: "lnbc250n1pjqxyz9pp5abc123def456ghi789jkl012mno345pqr678stu901vwx234yz567890abcdefghijklmnopqrstuvwxyz".to_string(),
            payment_hash: "fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321".to_string(),
            destination_pubkey: "03123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef01".to_string(),
            lnurl_pay_info: withdraw_metadata.lnurl_pay_info.clone(),
            lnurl_withdraw_info: withdraw_metadata.lnurl_withdraw_info.clone(),
            lnurl_receive_metadata: None,
        }),
        conversion_details: None,
    };

    // Test 8: Lightning payment with minimal details
    let lightning_minimal_payment = Payment {
        id: "lightning_minimal_pmt012".to_string(),
        payment_type: PaymentType::Receive,
        status: PaymentStatus::Failed,
        amount: 10_000,
        fees: 100,
        timestamp: Utc::now().timestamp().try_into().unwrap(),
        method: PaymentMethod::Lightning,
        details: Some(PaymentDetails::Lightning {
            description: None,
            preimage: None,
            invoice: "lnbc100n1pjqxyz9pp5def456ghi789jkl012mno345pqr678stu901vwx234yz567890abcdefghijklmnopqrstuvwxyz".to_string(),
            payment_hash: "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890".to_string(),
            destination_pubkey: "02987654321fedcba0987654321fedcba0987654321fedcba0987654321fedcba09".to_string(),
            lnurl_pay_info: None,
            lnurl_withdraw_info: None,
            lnurl_receive_metadata: None,
        }),
        conversion_details: None,
    };

    // Test 9: Lightning payment with LNURL receive metadata
    let lnurl_receive_payment_hash =
        "receivehash1234567890abcdef1234567890abcdef1234567890abcdef1234".to_string();
    let lnurl_receive_metadata = crate::LnurlReceiveMetadata {
        sender_comment: Some("Test sender comment".to_string()),
        nostr_zap_request: Some(r#"{"kind":9734,"content":"test zap"}"#.to_string()),
        nostr_zap_receipt: Some(r#"{"kind":9735,"content":"test receipt"}"#.to_string()),
    };
    let lightning_lnurl_receive_payment = Payment {
        id: "lightning_lnurl_receive_pmt".to_string(),
        payment_type: PaymentType::Receive,
        status: PaymentStatus::Completed,
        amount: 100_000,
        fees: 1000,
        timestamp: Utc::now().timestamp().try_into().unwrap(),
        method: PaymentMethod::Lightning,
        details: Some(PaymentDetails::Lightning {
            description: Some("LNURL receive test".to_string()),
            preimage: Some("receivepreimage1234567890abcdef1234567890abcdef1234567890abcdef12".to_string()),
            invoice: "lnbc1000n1pjqxyz9pp5receive123def456ghi789jkl012mno345pqr678stu901vwx234yz567890abcdefghijklmnopqrstuvwxyz".to_string(),
            payment_hash: lnurl_receive_payment_hash.clone(),
            destination_pubkey: "03receivepubkey123456789abcdef0123456789abcdef0123456789abcdef01234".to_string(),
            lnurl_pay_info: None,
            lnurl_withdraw_info: None,
            lnurl_receive_metadata: Some(lnurl_receive_metadata.clone()),
        }),
        conversion_details: None,
    };

    // Test 10: Withdraw payment
    let withdraw_payment = Payment {
        id: "withdraw_pmt345".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 200_000,
        fees: 2000,
        timestamp: Utc::now().timestamp().try_into().unwrap(),
        method: PaymentMethod::Withdraw,
        details: Some(PaymentDetails::Withdraw {
            tx_id: "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef12".to_string(),
        }),
        conversion_details: None,
    };

    // Test 11: Deposit payment
    let deposit_payment = Payment {
        id: "deposit_pmt678".to_string(),
        payment_type: PaymentType::Receive,
        status: PaymentStatus::Completed,
        amount: 150_000,
        fees: 1500,
        timestamp: Utc::now().timestamp().try_into().unwrap(),
        method: PaymentMethod::Deposit,
        details: Some(PaymentDetails::Deposit {
            tx_id: "fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321fe".to_string(),
        }),
        conversion_details: None,
    };

    // Test 12: Payment with no details
    let no_details_payment = Payment {
        id: "no_details_pmt901".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Pending,
        amount: 75_000,
        fees: 750,
        timestamp: Utc::now().timestamp().try_into().unwrap(),
        method: PaymentMethod::Unknown,
        details: None,
        conversion_details: None,
    };

    // Test 13: Successful conversion payment
    let successful_sent_conversion_payment_metadata = PaymentMetadata {
        parent_payment_id: Some("after_conversion_pmt124".to_string()),
        conversion_info: Some(crate::ConversionInfo {
            pool_id: "pool_abc".to_string(),
            conversion_id: "conversion_sent_pmt123".to_string(),
            status: crate::ConversionStatus::Completed,
            fee: Some(21),
            purpose: None,
        }),
        ..Default::default()
    };
    let successful_sent_conversion_payment = Payment {
        id: "conversion_sent_pmt123".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 10_000,
        fees: 0,
        timestamp: Utc::now().timestamp().try_into().unwrap(),
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: successful_sent_conversion_payment_metadata
                .conversion_info
                .clone(),
        }),
        conversion_details: None,
    };
    let successful_received_conversion_payment_metadata = PaymentMetadata {
        parent_payment_id: Some("after_conversion_pmt124".to_string()),
        ..Default::default()
    };
    let successful_received_conversion_payment = Payment {
        id: "conversion_received_pmt123".to_string(),
        payment_type: PaymentType::Receive,
        status: PaymentStatus::Completed,
        amount: 10_000_000,
        fees: 0,
        timestamp: Utc::now().timestamp().try_into().unwrap(),
        method: PaymentMethod::Token,
        details: Some(PaymentDetails::Token {
            metadata: token_metadata.clone(),
            tx_hash: "conversion_received_pmt123_tx_hash".to_string(),
            tx_type: TokenTransactionType::Transfer,
            invoice_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };
    let after_conversion_payment = Payment {
        id: "after_conversion_pmt124".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 10_000_000,
        fees: 0,
        timestamp: Utc::now().timestamp().try_into().unwrap(),
        method: PaymentMethod::Token,
        details: Some(PaymentDetails::Token {
            metadata: token_metadata.clone(),
            tx_hash: "after_conversion_pmt124_tx_hash".to_string(),
            tx_type: TokenTransactionType::Transfer,
            invoice_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    // Test 14: Failed conversion payment with refund info
    let failed_with_refund_conversion_payment_metadata = PaymentMetadata {
        conversion_info: Some(crate::ConversionInfo {
            pool_id: "pool_xyz".to_string(),
            conversion_id: "conversion_pmt789".to_string(),
            status: crate::ConversionStatus::Refunded,
            fee: None,
            purpose: None,
        }),
        ..Default::default()
    };
    let failed_with_refund_conversion_payment = Payment {
        id: "conversion_pmt789".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 10_000,
        fees: 0,
        timestamp: Utc::now().timestamp().try_into().unwrap(),
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: failed_with_refund_conversion_payment_metadata
                .conversion_info
                .clone(),
        }),
        conversion_details: None,
    };

    // Test 15: Failed conversion payment with no refund info
    let failed_no_refund_conversion_payment_metadata = PaymentMetadata {
        conversion_info: Some(crate::ConversionInfo {
            pool_id: "pool_xyz".to_string(),
            conversion_id: "conversion_pmt000".to_string(),
            status: crate::ConversionStatus::RefundNeeded,
            fee: None,
            purpose: None,
        }),
        ..Default::default()
    };
    let failed_no_refund_conversion_payment = Payment {
        id: "conversion_pmt000".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 20_000,
        fees: 0,
        timestamp: Utc::now().timestamp().try_into().unwrap(),
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: failed_no_refund_conversion_payment_metadata
                .conversion_info
                .clone(),
        }),
        conversion_details: None,
    };

    let test_payments = vec![
        spark_payment.clone(),
        spark_htlc_payment.clone(),
        token_transfer_payment.clone(),
        token_mint_payment.clone(),
        token_burn_payment.clone(),
        lightning_lnurl_pay_payment.clone(),
        lightning_lnurl_withdraw_payment.clone(),
        lightning_minimal_payment.clone(),
        lightning_lnurl_receive_payment.clone(),
        withdraw_payment.clone(),
        deposit_payment.clone(),
        no_details_payment.clone(),
        successful_sent_conversion_payment.clone(),
        successful_received_conversion_payment.clone(),
        after_conversion_payment.clone(),
        failed_with_refund_conversion_payment.clone(),
        failed_no_refund_conversion_payment.clone(),
    ];
    // Note: Storage layer returns related_payments as empty Vec.
    // The SDK layer is responsible for populating related_payments by calling
    // get_payments_by_parent_ids() and joining the results.
    // This test only verifies the Storage layer behavior.
    let test_related_payment_count = HashMap::from([(after_conversion_payment.id.clone(), 2)]);

    // Insert all payments
    for payment in &test_payments {
        storage.insert_payment(payment.clone()).await.unwrap();
    }
    storage
        .insert_payment_metadata(lightning_lnurl_pay_payment.id.clone(), pay_metadata)
        .await
        .unwrap();
    storage
        .insert_payment_metadata(
            lightning_lnurl_withdraw_payment.id.clone(),
            withdraw_metadata,
        )
        .await
        .unwrap();
    storage
        .set_lnurl_metadata(vec![SetLnurlMetadataItem {
            nostr_zap_receipt: lnurl_receive_metadata.nostr_zap_receipt.clone(),
            nostr_zap_request: lnurl_receive_metadata.nostr_zap_request.clone(),
            payment_hash: lnurl_receive_payment_hash.clone(),
            sender_comment: lnurl_receive_metadata.sender_comment.clone(),
        }])
        .await
        .unwrap();
    storage
        .insert_payment_metadata(
            successful_sent_conversion_payment.id.clone(),
            successful_sent_conversion_payment_metadata,
        )
        .await
        .unwrap();
    storage
        .insert_payment_metadata(
            successful_received_conversion_payment.id.clone(),
            successful_received_conversion_payment_metadata,
        )
        .await
        .unwrap();
    storage
        .insert_payment_metadata(
            failed_with_refund_conversion_payment.id.clone(),
            failed_with_refund_conversion_payment_metadata,
        )
        .await
        .unwrap();
    storage
        .insert_payment_metadata(
            failed_no_refund_conversion_payment.id.clone(),
            failed_no_refund_conversion_payment_metadata,
        )
        .await
        .unwrap();
    // List all payments (excludes child payments with parent_payment_id set)
    let payments = storage
        .list_payments(ListPaymentsRequest {
            offset: Some(0),
            limit: Some(20),
            ..Default::default()
        })
        .await
        .unwrap();
    // 17 total payments minus 2 child payments
    // (successful_sent_conversion_payment and successful_received_conversion_payment has parent_payment_id)
    assert_eq!(payments.len(), 15);

    // Test each payment type individually
    for (i, expected_payment) in test_payments.iter().enumerate() {
        let retrieved_payment = storage
            .get_payment_by_id(expected_payment.id.clone())
            .await
            .unwrap();

        // Basic fields
        assert_eq!(retrieved_payment.id, expected_payment.id);
        assert_eq!(
            retrieved_payment.payment_type,
            expected_payment.payment_type
        );
        assert_eq!(retrieved_payment.status, expected_payment.status);
        assert_eq!(retrieved_payment.amount, expected_payment.amount);
        assert_eq!(retrieved_payment.fees, expected_payment.fees);
        assert_eq!(retrieved_payment.method, expected_payment.method);

        // Storage layer always returns empty related_payments.
        // The SDK layer populates this field via get_payments_by_parent_ids().
        assert!(
            retrieved_payment.conversion_details.is_none(),
            "Storage layer should return an unset conversion_details for payment {}",
            expected_payment.id
        );

        // Test related payments retrieval
        let related_payment_count = storage
            .get_payments_by_parent_ids(vec![expected_payment.id.clone()])
            .await
            .unwrap()
            .get(&expected_payment.id)
            .map_or(0, Vec::len);
        let expected_related_count = test_related_payment_count
            .get(&expected_payment.id)
            .copied()
            .unwrap_or(0);
        assert_eq!(
            related_payment_count, expected_related_count,
            "Related payments count mismatch for payment {}",
            expected_payment.id
        );

        // Test payment details persistence
        match (&retrieved_payment.details, &expected_payment.details) {
            (None, None) => {}
            (
                Some(PaymentDetails::Spark {
                    invoice_details: r_invoice,
                    htlc_details: r_htlc,
                    conversion_info: r_conversion_info,
                }),
                Some(PaymentDetails::Spark {
                    invoice_details: e_invoice,
                    htlc_details: e_htlc,
                    conversion_info: e_conversion_info,
                }),
            ) => {
                assert_eq!(r_invoice, e_invoice);
                assert_eq!(r_htlc, e_htlc);
                assert_eq!(r_conversion_info, e_conversion_info);
            }
            (
                Some(PaymentDetails::Token {
                    metadata: r_metadata,
                    tx_hash: r_tx_hash,
                    tx_type: r_tx_type,
                    invoice_details: r_invoice,
                    conversion_info: r_conversion_info,
                }),
                Some(PaymentDetails::Token {
                    metadata: e_metadata,
                    tx_hash: e_tx_hash,
                    tx_type: e_tx_type,
                    invoice_details: e_invoice,
                    conversion_info: e_conversion_info,
                }),
            ) => {
                assert_eq!(r_metadata.identifier, e_metadata.identifier);
                assert_eq!(r_metadata.issuer_public_key, e_metadata.issuer_public_key);
                assert_eq!(r_metadata.name, e_metadata.name);
                assert_eq!(r_metadata.ticker, e_metadata.ticker);
                assert_eq!(r_metadata.decimals, e_metadata.decimals);
                assert_eq!(r_metadata.max_supply, e_metadata.max_supply);
                assert_eq!(r_metadata.is_freezable, e_metadata.is_freezable);
                assert_eq!(r_tx_hash, e_tx_hash);
                assert_eq!(r_tx_type, e_tx_type);
                assert_eq!(r_invoice, e_invoice);
                assert_eq!(r_conversion_info, e_conversion_info);
            }
            (
                Some(PaymentDetails::Lightning {
                    description: r_description,
                    preimage: r_preimage,
                    invoice: r_invoice,
                    payment_hash: r_hash,
                    destination_pubkey: r_dest_pubkey,
                    lnurl_pay_info: r_pay_lnurl,
                    lnurl_withdraw_info: r_withdraw_lnurl,
                    lnurl_receive_metadata: r_receive_metadata,
                }),
                Some(PaymentDetails::Lightning {
                    description: e_description,
                    preimage: e_preimage,
                    invoice: e_invoice,
                    payment_hash: e_hash,
                    destination_pubkey: e_dest_pubkey,
                    lnurl_pay_info: e_pay_lnurl,
                    lnurl_withdraw_info: e_withdraw_lnurl,
                    lnurl_receive_metadata: e_receive_metadata,
                }),
            ) => {
                assert_eq!(r_description, e_description);
                assert_eq!(r_preimage, e_preimage);
                assert_eq!(r_invoice, e_invoice);
                assert_eq!(r_hash, e_hash);
                assert_eq!(r_dest_pubkey, e_dest_pubkey);

                // Test LNURL pay info if present
                match (r_pay_lnurl, e_pay_lnurl) {
                    (Some(r_info), Some(e_info)) => {
                        assert_eq!(r_info.ln_address, e_info.ln_address);
                        assert_eq!(r_info.comment, e_info.comment);
                        assert_eq!(r_info.domain, e_info.domain);
                        assert_eq!(r_info.metadata, e_info.metadata);
                    }
                    (None, None) => {}
                    _ => panic!(
                        "LNURL pay info mismatch for payment {}",
                        expected_payment.id
                    ),
                }

                // Test LNURL withdraw info if present
                match (r_withdraw_lnurl, e_withdraw_lnurl) {
                    (Some(r_info), Some(e_info)) => {
                        assert_eq!(r_info.withdraw_url, e_info.withdraw_url);
                    }
                    (None, None) => {}
                    _ => panic!(
                        "LNURL withdraw info mismatch for payment {}",
                        expected_payment.id
                    ),
                }

                // Test LNURL receive metadata if present
                match (r_receive_metadata, e_receive_metadata) {
                    (Some(r_info), Some(e_info)) => {
                        assert_eq!(r_info.nostr_zap_request, e_info.nostr_zap_request);
                        assert_eq!(r_info.sender_comment, e_info.sender_comment);
                    }
                    (None, None) => {}
                    _ => panic!(
                        "LNURL receive metadata mismatch for payment {}",
                        expected_payment.id
                    ),
                }
            }
            (
                Some(PaymentDetails::Withdraw { tx_id: r_tx_id }),
                Some(PaymentDetails::Withdraw { tx_id: e_tx_id }),
            )
            | (
                Some(PaymentDetails::Deposit { tx_id: r_tx_id }),
                Some(PaymentDetails::Deposit { tx_id: e_tx_id }),
            ) => {
                assert_eq!(r_tx_id, e_tx_id);
            }
            _ => panic!(
                "Payment details mismatch for payment {} (index {})",
                expected_payment.id, i
            ),
        }
    }

    // Test filtering by payment type
    let send_payments = payments
        .iter()
        .filter(|p| p.payment_type == PaymentType::Send)
        .count();
    let receive_payments = payments
        .iter()
        .filter(|p| p.payment_type == PaymentType::Receive)
        .count();
    // Send: 9 - 1 child (successful_sent_conversion_payment) = 8
    // Receive: 8 - 1 child (successful_received_conversion_payment) = 7
    assert_eq!(send_payments, 8); // spark, token_burn, lightning_lnurl_pay, withdraw, no_details, after_conversion, failed_with_refund, failed_no_refund
    assert_eq!(receive_payments, 7); // spark_htlc, token_transfer, token_mint, lightning_lnurl_withdraw, lightning_minimal, lightning_lnurl_receive, deposit

    // Test filtering by status
    let completed_payments = payments
        .iter()
        .filter(|p| p.status == PaymentStatus::Completed)
        .count();
    let pending_payments = payments
        .iter()
        .filter(|p| p.status == PaymentStatus::Pending)
        .count();
    let failed_payments = payments
        .iter()
        .filter(|p| p.status == PaymentStatus::Failed)
        .count();
    // 14 completed payments minus 2 child payments = 12
    // (successful_sent_conversion_payment and successful_received_conversion_payment both have parent_payment_id)
    assert_eq!(completed_payments, 12); // spark, spark_htlc, token_mint, token_burn, lightning_lnurl_pay, lightning_lnurl_withdraw, lightning_lnurl_receive, withdraw, deposit, after_conversion, failed_with_refund, failed_no_refund
    assert_eq!(pending_payments, 2); // token, no_details
    assert_eq!(failed_payments, 1); // lightning_minimal

    // Test filtering by method
    let lightning_count = payments
        .iter()
        .filter(|p| p.method == PaymentMethod::Lightning)
        .count();
    assert_eq!(lightning_count, 4); // lightning_lnurl_pay, lightning_lnurl_withdraw, lightning_minimal, lightning_lnurl_receive

    // Test 9: Lightning payment with lnurl receive metadata (zap request and sender comment)
    let lightning_zap_payment = Payment {
        id: "lightning_zap_pmt".to_string(),
        payment_type: PaymentType::Receive,
        status: PaymentStatus::Completed,
        amount: 100_000,
        fees: 1000,
        timestamp: Utc::now().timestamp().try_into().unwrap(),
        method: PaymentMethod::Lightning,
        details: Some(PaymentDetails::Lightning {
            description: Some("Zap payment".to_string()),
            preimage: Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef01".to_string()),
            invoice: "lnbc1000n1pjqxyz9pp5zap123def456ghi789jkl012mno345pqr678stu901vwx234yz567890abcdefghijklmnopqrstuvwxyz".to_string(),
            payment_hash: "zaphash1234567890abcdef1234567890abcdef1234567890abcdef12345678".to_string(),
            destination_pubkey: "03zappubkey123456789abcdef0123456789abcdef0123456789abcdef0123456701".to_string(),
            lnurl_pay_info: None,
            lnurl_withdraw_info: None,
            lnurl_receive_metadata: None,
        }),
        conversion_details: None,
    };

    storage
        .insert_payment(lightning_zap_payment.clone())
        .await
        .unwrap();

    // Add lnurl receive metadata for the zap payment
    storage
        .set_lnurl_metadata(vec![SetLnurlMetadataItem {
            payment_hash: "zaphash1234567890abcdef1234567890abcdef1234567890abcdef12345678"
                .to_string(),
            sender_comment: Some("Great content!".to_string()),
            nostr_zap_request: Some(
                r#"{"kind":9734,"content":"zap request","tags":[]}"#.to_string(),
            ),
            nostr_zap_receipt: Some(
                r#"{"kind":9735,"content":"zap receipt","tags":[]}"#.to_string(),
            ),
        }])
        .await
        .unwrap();

    // Retrieve the payment and verify lnurl receive metadata is present
    let retrieved_zap_payment = storage
        .get_payment_by_id(lightning_zap_payment.id.clone())
        .await
        .unwrap();

    match retrieved_zap_payment.details {
        Some(PaymentDetails::Lightning {
            lnurl_receive_metadata: Some(metadata),
            ..
        }) => {
            assert_eq!(
                metadata.sender_comment,
                Some("Great content!".to_string()),
                "Sender comment should match"
            );
            assert_eq!(
                metadata.nostr_zap_request,
                Some(r#"{"kind":9734,"content":"zap request","tags":[]}"#.to_string()),
                "Nostr zap request should match"
            );
            assert_eq!(
                metadata.nostr_zap_receipt,
                Some(r#"{"kind":9735,"content":"zap receipt","tags":[]}"#.to_string()),
                "Nostr zap receipt should match"
            );
        }
        _ => panic!("Expected Lightning payment with lnurl receive metadata"),
    }

    // Test 10: Add multiple lnurl receive metadata items at once
    let lightning_zap_payment2 = Payment {
        id: "lightning_zap_pmt2".to_string(),
        payment_type: PaymentType::Receive,
        status: PaymentStatus::Completed,
        amount: 50_000,
        fees: 500,
        timestamp: Utc::now().timestamp().try_into().unwrap(),
        method: PaymentMethod::Lightning,
        details: Some(PaymentDetails::Lightning {
            description: Some("Another zap".to_string()),
            preimage: None,
            invoice: "lnbc500n1pjqxyz9pp5zap2".to_string(),
            payment_hash: "zaphash2".to_string(),
            destination_pubkey: "03zappubkey2".to_string(),
            lnurl_pay_info: None,
            lnurl_withdraw_info: None,
            lnurl_receive_metadata: None,
        }),
        conversion_details: None,
    };

    let lightning_zap_payment3 = Payment {
        id: "lightning_zap_pmt3".to_string(),
        payment_type: PaymentType::Receive,
        status: PaymentStatus::Completed,
        amount: 25_000,
        fees: 250,
        timestamp: Utc::now().timestamp().try_into().unwrap(),
        method: PaymentMethod::Lightning,
        details: Some(PaymentDetails::Lightning {
            description: Some("Third zap".to_string()),
            preimage: None,
            invoice: "lnbc250n1pjqxyz9pp5zap3".to_string(),
            payment_hash: "zaphash3".to_string(),
            destination_pubkey: "03zappubkey3".to_string(),
            lnurl_pay_info: None,
            lnurl_withdraw_info: None,
            lnurl_receive_metadata: None,
        }),
        conversion_details: None,
    };

    storage
        .insert_payment(lightning_zap_payment2.clone())
        .await
        .unwrap();
    storage
        .insert_payment(lightning_zap_payment3.clone())
        .await
        .unwrap();

    // Add multiple metadata items at once
    storage
        .set_lnurl_metadata(vec![
            SetLnurlMetadataItem {
                payment_hash: "zaphash2".to_string(),
                sender_comment: Some("Nice work!".to_string()),
                nostr_zap_request: None,
                nostr_zap_receipt: None,
            },
            SetLnurlMetadataItem {
                payment_hash: "zaphash3".to_string(),
                sender_comment: None,
                nostr_zap_request: Some(r#"{"kind":9734,"content":"zap3"}"#.to_string()),
                nostr_zap_receipt: None,
            },
        ])
        .await
        .unwrap();

    // Verify both payments have their respective metadata
    let retrieved_zap2 = storage
        .get_payment_by_id(lightning_zap_payment2.id.clone())
        .await
        .unwrap();

    match retrieved_zap2.details {
        Some(PaymentDetails::Lightning {
            lnurl_receive_metadata: Some(metadata),
            ..
        }) => {
            assert_eq!(
                metadata.sender_comment,
                Some("Nice work!".to_string()),
                "Second payment should have sender comment"
            );
            assert_eq!(
                metadata.nostr_zap_request, None,
                "Second payment should not have zap request"
            );
        }
        _ => panic!("Expected Lightning payment with lnurl receive metadata"),
    }

    let retrieved_zap3 = storage
        .get_payment_by_id(lightning_zap_payment3.id.clone())
        .await
        .unwrap();

    match retrieved_zap3.details {
        Some(PaymentDetails::Lightning {
            lnurl_receive_metadata: Some(metadata),
            ..
        }) => {
            assert_eq!(
                metadata.sender_comment, None,
                "Third payment should not have sender comment"
            );
            assert_eq!(
                metadata.nostr_zap_request,
                Some(r#"{"kind":9734,"content":"zap3"}"#.to_string()),
                "Third payment should have zap request"
            );
        }
        _ => panic!("Expected Lightning payment with lnurl receive metadata"),
    }

    // Test 11: Lightning payment without lnurl receive metadata should return None
    let retrieved_minimal = storage
        .get_payment_by_id(lightning_minimal_payment.id.clone())
        .await
        .unwrap();

    match retrieved_minimal.details {
        Some(PaymentDetails::Lightning {
            lnurl_receive_metadata,
            ..
        }) => {
            assert!(
                lnurl_receive_metadata.is_none(),
                "Payment without metadata should have None"
            );
        }
        _ => panic!("Expected Lightning payment"),
    }
}

pub async fn test_unclaimed_deposits_crud(storage: Box<dyn Storage>) {
    // Initially, list should be empty
    let deposits = storage.list_deposits().await.unwrap();
    assert_eq!(deposits.len(), 0);

    // Add first deposit
    storage
        .add_deposit("tx123".to_string(), 0, 50000)
        .await
        .unwrap();
    let deposits = storage.list_deposits().await.unwrap();
    assert_eq!(deposits.len(), 1);
    assert_eq!(deposits[0].txid, "tx123");
    assert_eq!(deposits[0].vout, 0);
    assert_eq!(deposits[0].amount_sats, 50000);
    assert!(deposits[0].claim_error.is_none());

    // Add second deposit
    storage
        .add_deposit("tx456".to_string(), 1, 75000)
        .await
        .unwrap();
    storage
        .update_deposit(
            "tx456".to_string(),
            1,
            UpdateDepositPayload::ClaimError {
                error: DepositClaimError::Generic {
                    message: "Test error".to_string(),
                },
            },
        )
        .await
        .unwrap();
    let deposits = storage.list_deposits().await.unwrap();
    assert_eq!(deposits.len(), 2);

    // Find deposit2 in the list
    let deposit2_found = deposits.iter().find(|d| d.txid == "tx456").unwrap();
    assert_eq!(deposit2_found.vout, 1);
    assert_eq!(deposit2_found.amount_sats, 75000);
    assert!(deposit2_found.claim_error.is_some());

    // Remove first deposit
    storage
        .delete_deposit("tx123".to_string(), 0)
        .await
        .unwrap();
    let deposits = storage.list_deposits().await.unwrap();
    assert_eq!(deposits.len(), 1);
    assert_eq!(deposits[0].txid, "tx456");

    // Remove second deposit
    storage
        .delete_deposit("tx456".to_string(), 1)
        .await
        .unwrap();
    let deposits = storage.list_deposits().await.unwrap();
    assert_eq!(deposits.len(), 0);
}

pub async fn test_deposit_refunds(storage: Box<dyn Storage>) {
    // Add the initial deposit
    storage
        .add_deposit("test_tx_123".to_string(), 0, 100_000)
        .await
        .unwrap();
    let deposits = storage.list_deposits().await.unwrap();
    assert_eq!(deposits.len(), 1);
    assert_eq!(deposits[0].txid, "test_tx_123");
    assert_eq!(deposits[0].vout, 0);
    assert_eq!(deposits[0].amount_sats, 100_000);
    assert!(deposits[0].claim_error.is_none());

    // Update the deposit refund information
    storage
        .update_deposit(
            "test_tx_123".to_string(),
            0,
            UpdateDepositPayload::Refund {
                refund_txid: "refund_tx_id_456".to_string(),
                refund_tx: "0200000001abcd1234...".to_string(),
            },
        )
        .await
        .unwrap();

    // Verify that the deposit information remains unchanged
    let deposits = storage.list_deposits().await.unwrap();
    assert_eq!(deposits.len(), 1);
    assert_eq!(deposits[0].txid, "test_tx_123");
    assert_eq!(deposits[0].vout, 0);
    assert_eq!(deposits[0].amount_sats, 100_000);
    assert!(deposits[0].claim_error.is_none());
    assert_eq!(
        deposits[0].refund_tx_id,
        Some("refund_tx_id_456".to_string())
    );
    assert_eq!(
        deposits[0].refund_tx,
        Some("0200000001abcd1234...".to_string())
    );
}

pub async fn test_payment_type_filtering(storage: Box<dyn Storage>) {
    // Create test payments with different types
    let send_payment = Payment {
        id: "send_1".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 10_000,
        fees: 100,
        timestamp: 1000,
        method: PaymentMethod::Lightning,
        details: Some(PaymentDetails::Lightning {
            invoice: "lnbc1".to_string(),
            payment_hash: "hash1".to_string(),
            destination_pubkey: "pubkey1".to_string(),
            description: None,
            preimage: None,
            lnurl_pay_info: None,
            lnurl_withdraw_info: None,
            lnurl_receive_metadata: None,
        }),
        conversion_details: None,
    };

    let receive_payment = Payment {
        id: "receive_1".to_string(),
        payment_type: PaymentType::Receive,
        status: PaymentStatus::Completed,
        amount: 20_000,
        fees: 200,
        timestamp: 2000,
        method: PaymentMethod::Lightning,
        details: Some(PaymentDetails::Lightning {
            invoice: "lnbc2".to_string(),
            payment_hash: "hash2".to_string(),
            destination_pubkey: "pubkey2".to_string(),
            description: None,
            preimage: None,
            lnurl_pay_info: None,
            lnurl_withdraw_info: None,
            lnurl_receive_metadata: None,
        }),
        conversion_details: None,
    };

    storage.insert_payment(send_payment).await.unwrap();
    storage.insert_payment(receive_payment).await.unwrap();

    // Test filter by Send type only
    let send_only = storage
        .list_payments(ListPaymentsRequest {
            type_filter: Some(vec![PaymentType::Send]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(send_only.len(), 1);
    assert_eq!(send_only[0].id, "send_1");

    // Test filter by Receive type only
    let receive_only = storage
        .list_payments(ListPaymentsRequest {
            type_filter: Some(vec![PaymentType::Receive]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(receive_only.len(), 1);
    assert_eq!(receive_only[0].id, "receive_1");

    // Test filter by both types
    let both_types = storage
        .list_payments(ListPaymentsRequest {
            type_filter: Some(vec![PaymentType::Send, PaymentType::Receive]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(both_types.len(), 2);

    // Test with no filter (should return all)
    let all_payments = storage
        .list_payments(ListPaymentsRequest::default())
        .await
        .unwrap();
    assert_eq!(all_payments.len(), 2);
}

pub async fn test_payment_status_filtering(storage: Box<dyn Storage>) {
    // Create test payments with different statuses
    let completed_payment = Payment {
        id: "completed_1".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 10_000,
        fees: 100,
        timestamp: 1000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    let pending_payment = Payment {
        id: "pending_1".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Pending,
        amount: 20_000,
        fees: 200,
        timestamp: 2000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    let failed_payment = Payment {
        id: "failed_1".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Failed,
        amount: 30_000,
        fees: 300,
        timestamp: 3000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    storage.insert_payment(completed_payment).await.unwrap();
    storage.insert_payment(pending_payment).await.unwrap();
    storage.insert_payment(failed_payment).await.unwrap();

    // Test filter by Completed status only
    let completed_only = storage
        .list_payments(ListPaymentsRequest {
            status_filter: Some(vec![PaymentStatus::Completed]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(completed_only.len(), 1);
    assert_eq!(completed_only[0].id, "completed_1");

    // Test filter by Pending status only
    let pending_only = storage
        .list_payments(ListPaymentsRequest {
            status_filter: Some(vec![PaymentStatus::Pending]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(pending_only.len(), 1);
    assert_eq!(pending_only[0].id, "pending_1");

    // Test filter by multiple statuses
    let completed_or_failed = storage
        .list_payments(ListPaymentsRequest {
            status_filter: Some(vec![PaymentStatus::Completed, PaymentStatus::Failed]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(completed_or_failed.len(), 2);
}

#[allow(clippy::too_many_lines)]
pub async fn test_asset_filtering(storage: Box<dyn Storage>) {
    use crate::models::TokenMetadata;

    // Create payments with different asset types
    let spark_payment = Payment {
        id: "spark_1".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 10_000,
        fees: 100,
        timestamp: 1000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    let lightning_payment = Payment {
        id: "lightning_1".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 20_000,
        fees: 200,
        timestamp: 2000,
        method: PaymentMethod::Lightning,
        details: Some(PaymentDetails::Lightning {
            invoice: "lnbc1".to_string(),
            payment_hash: "hash1".to_string(),
            destination_pubkey: "pubkey1".to_string(),
            description: None,
            preimage: None,
            lnurl_pay_info: None,
            lnurl_withdraw_info: None,
            lnurl_receive_metadata: None,
        }),
        conversion_details: None,
    };

    let token_payment = Payment {
        id: "token_1".to_string(),
        payment_type: PaymentType::Receive,
        status: PaymentStatus::Completed,
        amount: 30_000,
        fees: 300,
        timestamp: 3000,
        method: PaymentMethod::Token,
        details: Some(PaymentDetails::Token {
            metadata: TokenMetadata {
                identifier: "token_id_1".to_string(),
                issuer_public_key: "pubkey".to_string(),
                name: "Token 1".to_string(),
                ticker: "TK1".to_string(),
                decimals: 8,
                max_supply: 1_000_000,
                is_freezable: false,
            },
            tx_hash: "tx_hash_1".to_string(),
            tx_type: TokenTransactionType::Transfer,
            invoice_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    let withdraw_payment = Payment {
        id: "withdraw_1".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 40_000,
        fees: 400,
        timestamp: 4000,
        method: PaymentMethod::Withdraw,
        details: Some(PaymentDetails::Withdraw {
            tx_id: "withdraw_tx_1".to_string(),
        }),
        conversion_details: None,
    };

    let deposit_payment = Payment {
        id: "deposit_1".to_string(),
        payment_type: PaymentType::Receive,
        status: PaymentStatus::Completed,
        amount: 50_000,
        fees: 500,
        timestamp: 5000,
        method: PaymentMethod::Deposit,
        details: Some(PaymentDetails::Deposit {
            tx_id: "deposit_tx_1".to_string(),
        }),
        conversion_details: None,
    };

    storage.insert_payment(spark_payment).await.unwrap();
    storage.insert_payment(lightning_payment).await.unwrap();
    storage.insert_payment(token_payment).await.unwrap();
    storage.insert_payment(withdraw_payment).await.unwrap();
    storage.insert_payment(deposit_payment).await.unwrap();

    // Test filter by Bitcoin
    let spark_only = storage
        .list_payments(ListPaymentsRequest {
            asset_filter: Some(crate::AssetFilter::Bitcoin),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(spark_only.len(), 4);

    // Test filter by Token (no identifier)
    let token_only = storage
        .list_payments(ListPaymentsRequest {
            asset_filter: Some(crate::AssetFilter::Token {
                token_identifier: None,
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(token_only.len(), 1);
    assert_eq!(token_only[0].id, "token_1");

    // Test filter by Token with specific identifier
    let token_specific = storage
        .list_payments(ListPaymentsRequest {
            asset_filter: Some(crate::AssetFilter::Token {
                token_identifier: Some("token_id_1".to_string()),
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(token_specific.len(), 1);
    assert_eq!(token_specific[0].id, "token_1");

    // Test filter by Token with non-existent identifier
    let token_no_match = storage
        .list_payments(ListPaymentsRequest {
            asset_filter: Some(crate::AssetFilter::Token {
                token_identifier: Some("nonexistent".to_string()),
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(token_no_match.len(), 0);
}

#[allow(clippy::too_many_lines)]
pub async fn test_spark_htlc_status_filtering(storage: Box<dyn Storage>) {
    // Create payments with different HTLC statuses
    let htlc_waiting = Payment {
        id: "htlc_waiting".to_string(),
        payment_type: PaymentType::Receive,
        status: PaymentStatus::Pending,
        amount: 10_000,
        fees: 0,
        timestamp: 1000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: Some(SparkHtlcDetails {
                payment_hash: "hash1".to_string(),
                preimage: None,
                expiry_time: 2000,
                status: SparkHtlcStatus::WaitingForPreimage,
            }),
            conversion_info: None,
        }),
        conversion_details: None,
    };

    let htlc_shared = Payment {
        id: "htlc_shared".to_string(),
        payment_type: PaymentType::Receive,
        status: PaymentStatus::Completed,
        amount: 20_000,
        fees: 0,
        timestamp: 2000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: Some(SparkHtlcDetails {
                payment_hash: "hash2".to_string(),
                preimage: Some("preimage123".to_string()),
                expiry_time: 3000,
                status: SparkHtlcStatus::PreimageShared,
            }),
            conversion_info: None,
        }),
        conversion_details: None,
    };

    let htlc_returned = Payment {
        id: "htlc_returned".to_string(),
        payment_type: PaymentType::Receive,
        status: PaymentStatus::Failed,
        amount: 30_000,
        fees: 0,
        timestamp: 3000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: Some(SparkHtlcDetails {
                payment_hash: "hash3".to_string(),
                preimage: None,
                expiry_time: 4000,
                status: SparkHtlcStatus::Returned,
            }),
            conversion_info: None,
        }),
        conversion_details: None,
    };

    // Create a payment that is not HTLC-related
    let non_htlc_payment = Payment {
        id: "non_htlc".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 40_000,
        fees: 100,
        timestamp: 4000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: Some(crate::SparkInvoicePaymentDetails {
                description: Some("Test invoice".to_string()),
                invoice: "spark_invoice".to_string(),
            }),
            htlc_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    // Insert all payments
    storage.insert_payment(htlc_waiting).await.unwrap();
    storage.insert_payment(htlc_shared).await.unwrap();
    storage.insert_payment(htlc_returned).await.unwrap();
    storage.insert_payment(non_htlc_payment).await.unwrap();

    // Test filter for WaitingForPreimage
    let waiting_filter = storage
        .list_payments(ListPaymentsRequest {
            payment_details_filter: Some(vec![crate::PaymentDetailsFilter::Spark {
                htlc_status: Some(vec![SparkHtlcStatus::WaitingForPreimage]),
                conversion_refund_needed: None,
            }]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(waiting_filter.len(), 1);
    assert_eq!(waiting_filter[0].id, "htlc_waiting");

    // Test filter for PreimageShared
    let shared_filter = storage
        .list_payments(ListPaymentsRequest {
            payment_details_filter: Some(vec![crate::PaymentDetailsFilter::Spark {
                htlc_status: Some(vec![SparkHtlcStatus::PreimageShared]),
                conversion_refund_needed: None,
            }]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(shared_filter.len(), 1);
    assert_eq!(shared_filter[0].id, "htlc_shared");

    // Test filter for Returned
    let returned_filter = storage
        .list_payments(ListPaymentsRequest {
            payment_details_filter: Some(vec![crate::PaymentDetailsFilter::Spark {
                htlc_status: Some(vec![SparkHtlcStatus::Returned]),
                conversion_refund_needed: None,
            }]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(returned_filter.len(), 1);
    assert_eq!(returned_filter[0].id, "htlc_returned");

    // Test filter for multiple statuses (WaitingForPreimage and PreimageShared)
    let multiple_filter = storage
        .list_payments(ListPaymentsRequest {
            payment_details_filter: Some(vec![crate::PaymentDetailsFilter::Spark {
                htlc_status: Some(vec![
                    SparkHtlcStatus::WaitingForPreimage,
                    SparkHtlcStatus::PreimageShared,
                ]),
                conversion_refund_needed: None,
            }]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(multiple_filter.len(), 2);
    assert!(multiple_filter.iter().any(|p| p.id == "htlc_waiting"));
    assert!(multiple_filter.iter().any(|p| p.id == "htlc_shared"));

    // Test that non-HTLC payment is not included in any HTLC status filter
    let all_htlc_filter = storage
        .list_payments(ListPaymentsRequest {
            payment_details_filter: Some(vec![crate::PaymentDetailsFilter::Spark {
                htlc_status: Some(vec![
                    SparkHtlcStatus::WaitingForPreimage,
                    SparkHtlcStatus::PreimageShared,
                    SparkHtlcStatus::Returned,
                ]),
                conversion_refund_needed: None,
            }]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(all_htlc_filter.len(), 3);
    assert!(all_htlc_filter.iter().all(|p| p.id != "non_htlc"));
}

#[allow(clippy::too_many_lines)]
pub async fn test_conversion_refund_needed_filtering(storage: Box<dyn Storage>) {
    // Create payments with and without conversion info
    let payment_with_refund_metadata = PaymentMetadata {
        conversion_info: Some(crate::ConversionInfo {
            pool_id: "pool1".to_string(),
            conversion_id: "with_refund".to_string(),
            status: crate::ConversionStatus::Refunded,
            fee: None,
            purpose: None,
        }),
        ..Default::default()
    };
    let payment_with_refund = Payment {
        id: "with_refund".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 10_000_000,
        fees: 0,
        timestamp: 1000,
        method: PaymentMethod::Token,
        details: Some(PaymentDetails::Token {
            metadata: crate::TokenMetadata {
                identifier: "token1".to_string(),
                issuer_public_key: "pubkey1".to_string(),
                name: "Test Token".to_string(),
                ticker: "TTK".to_string(),
                decimals: 8,
                max_supply: 1_000_000_000,
                is_freezable: false,
            },
            tx_hash: "txhash1".to_string(),
            tx_type: TokenTransactionType::Transfer,
            invoice_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    let successful_conversion_metadata = PaymentMetadata {
        conversion_info: Some(crate::ConversionInfo {
            pool_id: "pool1".to_string(),
            conversion_id: "successful_conversion".to_string(),
            status: crate::ConversionStatus::Completed,
            fee: Some(100),
            purpose: None,
        }),
        ..Default::default()
    };
    let successful_conversion = Payment {
        id: "successful_conversion".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 20_000,
        fees: 0,
        timestamp: 2000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    let payment_without_refund_metadata = PaymentMetadata {
        conversion_info: Some(crate::ConversionInfo {
            pool_id: "pool1".to_string(),
            conversion_id: "without_refund".to_string(),
            status: crate::ConversionStatus::RefundNeeded,
            fee: None,
            purpose: None,
        }),
        ..Default::default()
    };
    let payment_without_refund = Payment {
        id: "without_refund".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 10_000,
        fees: 0,
        timestamp: 3000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    storage.insert_payment(payment_with_refund).await.unwrap();
    storage.insert_payment(successful_conversion).await.unwrap();
    storage
        .insert_payment(payment_without_refund)
        .await
        .unwrap();
    storage
        .insert_payment_metadata("with_refund".to_string(), payment_with_refund_metadata)
        .await
        .unwrap();
    storage
        .insert_payment_metadata(
            "successful_conversion".to_string(),
            successful_conversion_metadata,
        )
        .await
        .unwrap();
    storage
        .insert_payment_metadata(
            "without_refund".to_string(),
            payment_without_refund_metadata,
        )
        .await
        .unwrap();

    let payments = storage
        .list_payments(ListPaymentsRequest::default())
        .await
        .unwrap();
    assert_eq!(payments.len(), 3);

    // Test filter for payments missing transfer refund info
    let missing_refund_filter = storage
        .list_payments(ListPaymentsRequest {
            payment_details_filter: Some(vec![crate::PaymentDetailsFilter::Spark {
                htlc_status: None,
                conversion_refund_needed: Some(true),
            }]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(missing_refund_filter.len(), 1);
    assert_eq!(missing_refund_filter[0].id, "without_refund");

    // Test filter for payments with transfer refund info present
    let present_refund_filter = storage
        .list_payments(ListPaymentsRequest {
            payment_details_filter: Some(vec![crate::PaymentDetailsFilter::Token {
                conversion_refund_needed: Some(false),
                tx_hash: None,
                tx_type: None,
            }]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(present_refund_filter.len(), 1);
    assert_eq!(present_refund_filter[0].id, "with_refund");

    // Test multiple payment detail filters
    let multiple_filters = storage
        .list_payments(ListPaymentsRequest {
            payment_details_filter: Some(vec![
                crate::PaymentDetailsFilter::Spark {
                    htlc_status: None,
                    conversion_refund_needed: Some(true),
                },
                crate::PaymentDetailsFilter::Token {
                    conversion_refund_needed: Some(false),
                    tx_hash: None,
                    tx_type: None,
                },
            ]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(multiple_filters.len(), 2);

    // Test filter for token payments missing transfer refund info
    let token_no_refund_filter = storage
        .list_payments(ListPaymentsRequest {
            payment_details_filter: Some(vec![crate::PaymentDetailsFilter::Token {
                conversion_refund_needed: Some(true),
                tx_hash: None,
                tx_type: None,
            }]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(token_no_refund_filter.len(), 0);

    // Test filter for spark payments with transfer refund info present
    let spark_with_refund_filter = storage
        .list_payments(ListPaymentsRequest {
            payment_details_filter: Some(vec![crate::PaymentDetailsFilter::Spark {
                htlc_status: None,
                conversion_refund_needed: Some(false),
            }]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(spark_with_refund_filter.len(), 1);

    // Test filter for all payments regardless of transfer refund info
    let all_payments_filter = storage
        .list_payments(ListPaymentsRequest {
            payment_details_filter: Some(vec![crate::PaymentDetailsFilter::Spark {
                htlc_status: None,
                conversion_refund_needed: None,
            }]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(all_payments_filter.len(), 3);
}

#[allow(clippy::too_many_lines)]
pub async fn test_token_transaction_type_filtering(storage: Box<dyn Storage>) {
    let token_metadata = TokenMetadata {
        identifier: "token123".to_string(),
        issuer_public_key: "02abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890ab"
            .to_string(),
        name: "Test Token".to_string(),
        ticker: "TTK".to_string(),
        decimals: 8,
        max_supply: 21_000_000,
        is_freezable: false,
    };
    // Create payments with different transaction types
    let payment1 = Payment {
        id: "transfer_1".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 10_000,
        fees: 100,
        timestamp: 1000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Token {
            metadata: token_metadata.clone(),
            tx_hash: "tx_hash_transfer".to_string(),
            tx_type: TokenTransactionType::Transfer,
            invoice_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };
    let payment2 = Payment {
        id: "mint_2".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 20_000,
        fees: 200,
        timestamp: 2000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Token {
            metadata: token_metadata.clone(),
            tx_hash: "tx_hash_mint".to_string(),
            tx_type: TokenTransactionType::Mint,
            invoice_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };
    let payment3 = Payment {
        id: "burn_3".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 30_000,
        fees: 300,
        timestamp: 3000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Token {
            metadata: token_metadata.clone(),
            tx_hash: "tx_hash_burn".to_string(),
            tx_type: TokenTransactionType::Burn,
            invoice_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };
    storage.insert_payment(payment1).await.unwrap();
    storage.insert_payment(payment2).await.unwrap();
    storage.insert_payment(payment3).await.unwrap();

    // Test filter by transaction type
    let transfer_filter = storage
        .list_payments(ListPaymentsRequest {
            payment_details_filter: Some(vec![crate::PaymentDetailsFilter::Token {
                tx_type: Some(TokenTransactionType::Transfer),
                tx_hash: None,
                conversion_refund_needed: None,
            }]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(transfer_filter.len(), 1);
    assert_eq!(transfer_filter[0].id, "transfer_1");

    // Test filter by mint transaction type

    let mint_filter = storage
        .list_payments(ListPaymentsRequest {
            payment_details_filter: Some(vec![crate::PaymentDetailsFilter::Token {
                tx_type: Some(TokenTransactionType::Mint),
                tx_hash: None,
                conversion_refund_needed: None,
            }]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(mint_filter.len(), 1);
    assert_eq!(mint_filter[0].id, "mint_2");

    // Test filter by burn transaction type
    let burn_filter = storage
        .list_payments(ListPaymentsRequest {
            payment_details_filter: Some(vec![crate::PaymentDetailsFilter::Token {
                tx_type: Some(TokenTransactionType::Burn),
                tx_hash: None,
                conversion_refund_needed: None,
            }]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(burn_filter.len(), 1);
    assert_eq!(burn_filter[0].id, "burn_3");
}

pub async fn test_timestamp_filtering(storage: Box<dyn Storage>) {
    // Create payments at different timestamps
    let payment1 = Payment {
        id: "ts_1000".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 10_000,
        fees: 100,
        timestamp: 1000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    let payment2 = Payment {
        id: "ts_2000".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 20_000,
        fees: 200,
        timestamp: 2000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    let payment3 = Payment {
        id: "ts_3000".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 30_000,
        fees: 300,
        timestamp: 3000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    storage.insert_payment(payment1).await.unwrap();
    storage.insert_payment(payment2).await.unwrap();
    storage.insert_payment(payment3).await.unwrap();

    // Test filter by from_timestamp
    let from_2000 = storage
        .list_payments(ListPaymentsRequest {
            from_timestamp: Some(2000),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(from_2000.len(), 2);
    assert!(from_2000.iter().any(|p| p.id == "ts_2000"));
    assert!(from_2000.iter().any(|p| p.id == "ts_3000"));

    // Test filter by to_timestamp
    let to_2000 = storage
        .list_payments(ListPaymentsRequest {
            to_timestamp: Some(2000),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(to_2000.len(), 1);
    assert!(to_2000.iter().any(|p| p.id == "ts_1000"));

    // Test filter by both from_timestamp and to_timestamp
    let range = storage
        .list_payments(ListPaymentsRequest {
            from_timestamp: Some(1500),
            to_timestamp: Some(2500),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(range.len(), 1);
    assert_eq!(range[0].id, "ts_2000");
}

pub async fn test_combined_filters(storage: Box<dyn Storage>) {
    // Create diverse test payments
    let payment1 = Payment {
        id: "combined_1".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 10_000,
        fees: 100,
        timestamp: 1000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    let payment2 = Payment {
        id: "combined_2".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Pending,
        amount: 20_000,
        fees: 200,
        timestamp: 2000,
        method: PaymentMethod::Lightning,
        details: Some(PaymentDetails::Lightning {
            invoice: "lnbc1".to_string(),
            payment_hash: "hash1".to_string(),
            destination_pubkey: "pubkey1".to_string(),
            description: None,
            preimage: None,
            lnurl_pay_info: None,
            lnurl_withdraw_info: None,
            lnurl_receive_metadata: None,
        }),
        conversion_details: None,
    };

    let payment3 = Payment {
        id: "combined_3".to_string(),
        payment_type: PaymentType::Receive,
        status: PaymentStatus::Completed,
        amount: 30_000,
        fees: 300,
        timestamp: 3000,
        method: PaymentMethod::Lightning,
        details: Some(PaymentDetails::Lightning {
            invoice: "lnbc2".to_string(),
            payment_hash: "hash2".to_string(),
            destination_pubkey: "pubkey2".to_string(),
            description: None,
            preimage: None,
            lnurl_pay_info: None,
            lnurl_withdraw_info: None,
            lnurl_receive_metadata: None,
        }),
        conversion_details: None,
    };

    storage.insert_payment(payment1).await.unwrap();
    storage.insert_payment(payment2).await.unwrap();
    storage.insert_payment(payment3).await.unwrap();

    // Test: Send + Completed
    let send_completed = storage
        .list_payments(ListPaymentsRequest {
            type_filter: Some(vec![PaymentType::Send]),
            status_filter: Some(vec![PaymentStatus::Completed]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(send_completed.len(), 1);
    assert_eq!(send_completed[0].id, "combined_1");

    // Test: Bitcoin + timestamp range
    let bitcoin_recent = storage
        .list_payments(ListPaymentsRequest {
            asset_filter: Some(crate::AssetFilter::Bitcoin),
            from_timestamp: Some(2500),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(bitcoin_recent.len(), 1);
    assert_eq!(bitcoin_recent[0].id, "combined_3");

    // Test: Type + Status + Asset
    let send_pending_bitcoin = storage
        .list_payments(ListPaymentsRequest {
            type_filter: Some(vec![PaymentType::Send]),
            status_filter: Some(vec![PaymentStatus::Pending]),
            asset_filter: Some(crate::AssetFilter::Bitcoin),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(send_pending_bitcoin.len(), 1);
    assert_eq!(send_pending_bitcoin[0].id, "combined_2");
}

pub async fn test_sort_order(storage: Box<dyn Storage>) {
    // Create payments at different timestamps
    let payment1 = Payment {
        id: "sort_1".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 10_000,
        fees: 100,
        timestamp: 1000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    let payment2 = Payment {
        id: "sort_2".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 20_000,
        fees: 200,
        timestamp: 2000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    let payment3 = Payment {
        id: "sort_3".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 30_000,
        fees: 300,
        timestamp: 3000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };

    storage.insert_payment(payment1).await.unwrap();
    storage.insert_payment(payment2).await.unwrap();
    storage.insert_payment(payment3).await.unwrap();

    // Test default sort (descending by timestamp)
    let desc_payments = storage
        .list_payments(ListPaymentsRequest::default())
        .await
        .unwrap();
    assert_eq!(desc_payments.len(), 3);
    assert_eq!(desc_payments[0].id, "sort_3"); // Most recent first
    assert_eq!(desc_payments[1].id, "sort_2");
    assert_eq!(desc_payments[2].id, "sort_1");

    // Test ascending sort
    let asc_payments = storage
        .list_payments(ListPaymentsRequest {
            sort_ascending: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(asc_payments.len(), 3);
    assert_eq!(asc_payments[0].id, "sort_1"); // Oldest first
    assert_eq!(asc_payments[1].id, "sort_2");
    assert_eq!(asc_payments[2].id, "sort_3");

    // Test explicit descending sort
    let desc_explicit = storage
        .list_payments(ListPaymentsRequest {
            sort_ascending: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(desc_explicit.len(), 3);
    assert_eq!(desc_explicit[0].id, "sort_3");
    assert_eq!(desc_explicit[1].id, "sort_2");
    assert_eq!(desc_explicit[2].id, "sort_1");
}

pub async fn test_payment_metadata(storage: Box<dyn Storage>) {
    let cache = ObjectCacheRepository::new(storage.into());

    // Prepare test data
    let payment_request1 = "pr1".to_string();
    let metadata1 = PaymentMetadata {
        lnurl_description: Some("desc1".to_string()),
        lnurl_withdraw_info: Some(LnurlWithdrawInfo {
            withdraw_url: "https://callback.url".to_string(),
        }),
        ..Default::default()
    };

    let payment_request2 = "pr2".to_string();
    let metadata2 = PaymentMetadata {
        lnurl_description: Some("desc2".to_string()),
        lnurl_withdraw_info: Some(LnurlWithdrawInfo {
            withdraw_url: "https://callback2.url".to_string(),
        }),
        ..Default::default()
    };

    // set_payment_request_metadata
    cache
        .save_payment_metadata(&payment_request1, &metadata1)
        .await
        .unwrap();
    cache
        .save_payment_metadata(&payment_request2, &metadata2)
        .await
        .unwrap();

    // get_payment_request_metadata
    let fetched1 = cache
        .fetch_payment_metadata(&payment_request1)
        .await
        .unwrap();
    assert!(fetched1.is_some());
    let fetched1 = fetched1.unwrap();
    assert_eq!(fetched1.lnurl_description.unwrap(), "desc1");
    assert_eq!(
        fetched1.lnurl_withdraw_info.unwrap().withdraw_url,
        "https://callback.url"
    );

    let fetched2 = cache
        .fetch_payment_metadata(&payment_request2)
        .await
        .unwrap();
    assert!(fetched2.is_some());

    // delete_payment_request_metadata
    cache
        .delete_payment_metadata(&payment_request1)
        .await
        .unwrap();
    let deleted = cache
        .fetch_payment_metadata(&payment_request1)
        .await
        .unwrap();
    assert!(deleted.is_none());
}

pub async fn test_payment_details_update_persistence(storage: Box<dyn Storage>) {
    // Create a payment with incomplete details
    let mut payment = Payment {
        id: "payment_1".to_string(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Pending,
        amount: 15_000,
        fees: 150,
        timestamp: 1_234_567_890,
        method: PaymentMethod::Lightning,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: Some(SparkHtlcDetails {
                payment_hash: "hash_123".to_string(),
                preimage: None,
                expiry_time: 1_234_567_990,
                status: SparkHtlcStatus::WaitingForPreimage,
            }),
            conversion_info: None,
        }),
        conversion_details: None,
    };

    // Insert the payment into storage
    storage.insert_payment(payment.clone()).await.unwrap();

    // Simulate payment completion by updating status
    payment.status = PaymentStatus::Completed;
    storage.insert_payment(payment.clone()).await.unwrap();

    // Check the payment details
    let updated_payment = storage
        .get_payment_by_id("payment_1".to_string())
        .await
        .unwrap();
    assert_eq!(updated_payment.status, PaymentStatus::Completed);
    let Some(PaymentDetails::Spark { htlc_details, .. }) = &updated_payment.details else {
        panic!("Payment details are not of Spark type");
    };
    assert_eq!(
        htlc_details.as_ref().unwrap().status,
        SparkHtlcStatus::WaitingForPreimage
    );

    // Now, update the payment details
    payment.details = Some(PaymentDetails::Spark {
        invoice_details: None,
        htlc_details: Some(SparkHtlcDetails {
            payment_hash: "hash_123".to_string(),
            preimage: Some("preimage_123".to_string()),
            expiry_time: 1_234_567_990,
            status: SparkHtlcStatus::PreimageShared,
        }),
        conversion_info: None,
    });
    storage.insert_payment(payment.clone()).await.unwrap();

    // Check the updated payment details
    let updated_payment = storage
        .get_payment_by_id("payment_1".to_string())
        .await
        .unwrap();
    let Some(PaymentDetails::Spark { htlc_details, .. }) = &updated_payment.details else {
        panic!("Payment details are not of Spark type");
    };
    assert_eq!(
        htlc_details.as_ref().unwrap().status,
        SparkHtlcStatus::PreimageShared
    );
    assert_eq!(
        htlc_details.as_ref().unwrap().preimage.as_ref().unwrap(),
        "preimage_123"
    );
}

/// Tests that `insert_payment_metadata` preserves existing fields when updating with partial data.
/// This verifies the COALESCE behavior in the SQL upsert.
pub async fn test_payment_metadata_merge(storage: Box<dyn Storage>) {
    let payment_id = "merge_test_payment".to_string();
    let parent_id = "parent_payment_456".to_string();

    // Create the payment first so we can fetch it via get_payment_by_id
    let payment = Payment {
        id: payment_id.clone(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 1000,
        fees: 10,
        timestamp: 1_700_000_000,
        method: PaymentMethod::Spark,
        details: Some(PaymentDetails::Spark {
            invoice_details: None,
            htlc_details: None,
            conversion_info: None,
        }),
        conversion_details: None,
    };
    storage.insert_payment(payment).await.unwrap();

    // Create the parent payment so get_payments_by_parent_ids works
    let parent_payment = Payment {
        id: parent_id.clone(),
        payment_type: PaymentType::Send,
        status: PaymentStatus::Completed,
        amount: 2000,
        fees: 20,
        timestamp: 1_700_000_001,
        method: PaymentMethod::Spark,
        details: None,
        conversion_details: None,
    };
    storage.insert_payment(parent_payment).await.unwrap();

    // Step 1: Set metadata with only conversion_info
    let metadata1 = PaymentMetadata {
        conversion_info: Some(crate::ConversionInfo {
            pool_id: "pool_123".to_string(),
            conversion_id: "conv_123".to_string(),
            status: crate::ConversionStatus::Completed,
            fee: Some(100),
            purpose: None,
        }),
        ..Default::default()
    };
    storage
        .insert_payment_metadata(payment_id.clone(), metadata1)
        .await
        .unwrap();

    // Verify conversion_info is set via get_payment_by_id
    let fetched = storage.get_payment_by_id(payment_id.clone()).await.unwrap();
    let Some(PaymentDetails::Spark {
        conversion_info, ..
    }) = &fetched.details
    else {
        panic!("Expected Spark payment details");
    };
    assert!(conversion_info.is_some());
    assert_eq!(conversion_info.as_ref().unwrap().conversion_id, "conv_123");

    // Step 2: Set metadata with only parent_payment_id (conversion_info is None)
    let metadata2 = PaymentMetadata {
        parent_payment_id: Some(parent_id.clone()),
        ..Default::default()
    };
    storage
        .insert_payment_metadata(payment_id.clone(), metadata2)
        .await
        .unwrap();

    // Verify parent_payment_id was set via get_payments_by_parent_ids
    let related = storage
        .get_payments_by_parent_ids(vec![parent_id.clone()])
        .await
        .unwrap();
    assert!(
        related.contains_key(&parent_id),
        "parent_payment_id should be set"
    );
    assert_eq!(related.get(&parent_id).unwrap().len(), 1);
    assert_eq!(related.get(&parent_id).unwrap()[0].id, payment_id);

    // Verify conversion_info is STILL present (not cleared by partial update)
    let fetched = storage.get_payment_by_id(payment_id.clone()).await.unwrap();
    let Some(PaymentDetails::Spark {
        conversion_info, ..
    }) = &fetched.details
    else {
        panic!("Expected Spark payment details");
    };
    assert!(
        conversion_info.is_some(),
        "conversion_info should be preserved, not cleared by partial update"
    );
    assert_eq!(conversion_info.as_ref().unwrap().conversion_id, "conv_123");
}
