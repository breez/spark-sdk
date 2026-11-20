use platform_utils::time::UNIX_EPOCH;
use spark_primitives::{
    FinalizeTokenInvoiceRequest, PrepareTokenInvoiceRequest, PreparedTokenInvoice,
    finalize_token_invoice, prepare_token_invoice,
};

use crate::{
    Network,
    address::{SparkAddress, TokensPayment, error::AddressError, sign_invoice},
    signer::{SparkInvoiceKind, SparkSigner},
    token::bech32m_decode_token_id,
    utils::byte_padding::BytePadding,
};

pub(crate) fn hash(
    address: &SparkAddress,
    payment: &TokensPayment,
) -> Result<Vec<u8>, AddressError> {
    Ok(prepare(address, payment)?.spark_invoice_hash)
}

pub(crate) async fn encode(
    address: &SparkAddress,
    payment: &TokensPayment,
    spark_signer: &dyn SparkSigner,
) -> Result<String, AddressError> {
    let prepared = prepare(address, payment)?;
    let signature = sign_invoice(
        spark_signer,
        SparkInvoiceKind::Tokens,
        &prepared.spark_invoice_hash,
    )
    .await?;
    finalize_token_invoice(FinalizeTokenInvoiceRequest {
        receiver_identity_public_key: address.identity_public_key.serialize().to_vec(),
        network: network_as_u32(address.network),
        spark_invoice_fields_bytes: prepared.spark_invoice_fields_bytes,
        signature: Some(signature.serialize().to_vec()),
    })
    .map_err(|e| AddressError::Other(format!("Failed to finalize token invoice: {e}")))
}

fn prepare(
    address: &SparkAddress,
    payment: &TokensPayment,
) -> Result<PreparedTokenInvoice, AddressError> {
    let invoice_fields = address
        .spark_invoice_fields
        .as_ref()
        .ok_or_else(|| AddressError::Other("No invoice fields".to_string()))?;

    let token_identifier = payment
        .token_identifier
        .as_deref()
        .map(|id| {
            bech32m_decode_token_id(id, None)
                .map_err(|e| AddressError::Other(format!("Invalid token identifier: {e}")))
        })
        .transpose()?;

    let token_amount = payment.amount.map(|a| a.to_unpadded_be_bytes());

    let sender_spark_address = invoice_fields
        .sender_public_key
        .map(|pk| SparkAddress::new(pk, address.network, None).to_address_string())
        .transpose()?;

    let expiry_time_unix_millis = invoice_fields.expiry_time.map(|t| {
        t.duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    });

    prepare_token_invoice(PrepareTokenInvoiceRequest {
        receiver_identity_public_key: address.identity_public_key.serialize().to_vec(),
        network: network_as_u32(address.network),
        token_identifier,
        token_amount,
        memo: invoice_fields.memo.clone(),
        sender_spark_address,
        expiry_time_unix_millis,
        invoice_id: Some(invoice_fields.id.as_bytes().to_vec()),
    })
    .map_err(|e| AddressError::Other(format!("Failed to prepare token invoice: {e}")))
}

fn network_as_u32(network: Network) -> u32 {
    u32::try_from(network.to_proto_network() as i32).expect("network proto value is non-negative")
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bitcoin::secp256k1::{PublicKey, Secp256k1};
    use macros::{async_test_all, test_all};
    use platform_utils::time::{SystemTime, UNIX_EPOCH};

    use crate::{
        Network,
        address::{SparkAddress, SparkAddressPaymentType, SparkInvoiceFields, TokensPayment},
        signer::{SparkSigner, SparkSignerAdapter, create_test_signer},
    };

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn create_test_public_key() -> PublicKey {
        let secp = Secp256k1::new();
        let (secret_key, _) = secp.generate_keypair(&mut bitcoin::secp256k1::rand::thread_rng());
        PublicKey::from_slice(&secret_key.public_key(&secp).serialize()).unwrap()
    }

    fn ms_aligned_now() -> SystemTime {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        UNIX_EPOCH + platform_utils::time::Duration::from_millis(since_epoch.as_millis() as u64)
    }

    #[test_all]
    fn test_parse_specific_token_invoice() {
        let invoice_str = "sparkrt1pgss8cf4gru7ece2ryn8ym3vm3yz8leeend2589m7svq2mgv0xncfyx8zfeqsqgjzqqe5pmtue38y3avl89vac53nkzj5prdv4kk7v3pqdur4y4c5nlcyr7lksm4mhrhdzakas9yt8gz4levtnfe49sgkqknywstprharhk8qcgvpz8vtudzvz3q5xy2yxnpacs2yl00fnajxylsljq9y0uesr6qylyxq2lxnum8r63pyqsraqdyp57uf363avdlv59eqjfdszpwc2y3zfpww2cevcx92zw40qxf5fedvrlmnrmsg7pa3egggtw03kd0rz73lvgl5u3c02krhhcwc3haec4q2e582x";
        let invoice = SparkAddress::from_str(invoice_str).unwrap();

        assert_eq!(invoice.network, Network::Regtest);
        let invoice_fields = invoice.spark_invoice_fields.unwrap();
        assert_eq!(
            invoice_fields.payment_type,
            Some(SparkAddressPaymentType::TokensPayment(TokensPayment {
                token_identifier: Some(
                    "btknrt15xy2yxnpacs2yl00fnajxylsljq9y0uesr6qylyxq2lxnum8r63qfues7q".to_string(),
                ),
                amount: Some(1000)
            }))
        );
        assert_eq!(
            invoice_fields
                .expiry_time
                .unwrap()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            1761061103
        );
        assert_eq!(invoice_fields.memo, Some("memo".to_string()));
        assert_eq!(
            invoice_fields.sender_public_key,
            Some(
                PublicKey::from_str(
                    "03783a92b8a4ff820fdfb4375ddc7768bb6ec0a459d02aff2c5cd39a9608b02d32"
                )
                .unwrap()
            )
        );
    }

    #[async_test_all]
    async fn test_invoice_roundtrip() {
        let signer = std::sync::Arc::new(create_test_signer());
        let spark_signer = SparkSignerAdapter::new(signer.clone());
        let public_key = spark_signer.get_identity_public_key().await.unwrap();
        let sender_public_key = create_test_public_key();
        let invoice_fields = SparkInvoiceFields {
            id: uuid::Uuid::now_v7(),
            version: 1,
            sender_public_key: Some(sender_public_key),
            expiry_time: Some(ms_aligned_now()),
            payment_type: Some(SparkAddressPaymentType::TokensPayment(TokensPayment {
                token_identifier: Some(
                    "btknrt15xy2yxnpacs2yl00fnajxylsljq9y0uesr6qylyxq2lxnum8r63qfues7q".to_string(),
                ),
                amount: Some(100),
            })),
            memo: Some("Test payment".to_string()),
        };

        let original_address =
            SparkAddress::new(public_key, Network::Regtest, Some(invoice_fields.clone()));

        let invoice_string = original_address
            .to_invoice_string(&spark_signer)
            .await
            .unwrap();
        let parsed_address = SparkAddress::from_str(&invoice_string).unwrap();

        assert_eq!(
            parsed_address.identity_public_key,
            original_address.identity_public_key
        );
        assert_eq!(parsed_address.network, original_address.network);

        // Check payment intent fields
        assert!(parsed_address.spark_invoice_fields.is_some());
        let parsed_invoice_fields = parsed_address.spark_invoice_fields.unwrap();
        let original_invoice_fields = original_address.spark_invoice_fields.unwrap();

        assert_eq!(parsed_invoice_fields.id, original_invoice_fields.id);
        assert_eq!(
            parsed_invoice_fields.expiry_time,
            original_invoice_fields.expiry_time
        );
        assert_eq!(parsed_invoice_fields.id, original_invoice_fields.id);
        assert_eq!(parsed_invoice_fields.memo, original_invoice_fields.memo);

        let Some(SparkAddressPaymentType::TokensPayment(tokens_payment1)) =
            parsed_invoice_fields.payment_type
        else {
            panic!("Expected TokensPayment");
        };
        let Some(SparkAddressPaymentType::TokensPayment(tokens_payment2)) =
            original_invoice_fields.payment_type
        else {
            panic!("Expected TokensPayment");
        };
        assert_eq!(
            tokens_payment1.token_identifier,
            tokens_payment2.token_identifier
        );
        assert_eq!(tokens_payment1.amount, tokens_payment2.amount);
    }

    #[async_test_all]
    async fn test_compare_addresses_with_and_without_invoice_fields() {
        let signer = std::sync::Arc::new(create_test_signer());
        let spark_signer = SparkSignerAdapter::new(signer.clone());
        let public_key = spark_signer.get_identity_public_key().await.unwrap();
        let sender_public_key = create_test_public_key();

        // Create address without invoice fields
        let address_without_intent = SparkAddress::new(public_key, Network::Mainnet, None);
        let string_without_intent = address_without_intent.to_address_string().unwrap();

        let invoice_fields = SparkInvoiceFields {
            id: uuid::Uuid::now_v7(),
            version: 1,
            sender_public_key: Some(sender_public_key),
            expiry_time: Some(ms_aligned_now()),
            payment_type: Some(SparkAddressPaymentType::TokensPayment(TokensPayment {
                token_identifier: Some(
                    "btknrt15xy2yxnpacs2yl00fnajxylsljq9y0uesr6qylyxq2lxnum8r63qfues7q".to_string(),
                ),
                amount: Some(100),
            })),
            memo: Some("Test memo".to_string()),
        };
        let address_with_intent =
            SparkAddress::new(public_key, Network::Mainnet, Some(invoice_fields));
        let string_with_intent = address_with_intent
            .to_invoice_string(&spark_signer)
            .await
            .unwrap();

        // The strings should be different due to the payment intent data
        assert_ne!(string_without_intent, string_with_intent);

        // Parse both addresses and verify the core data matches
        let parsed_without_intent = SparkAddress::from_str(&string_without_intent).unwrap();
        let parsed_with_intent = SparkAddress::from_str(&string_with_intent).unwrap();

        assert_eq!(
            parsed_without_intent.identity_public_key,
            parsed_with_intent.identity_public_key
        );
        assert_eq!(parsed_without_intent.network, parsed_with_intent.network);
        assert!(parsed_without_intent.spark_invoice_fields.is_none());
        assert!(parsed_with_intent.spark_invoice_fields.is_some());
    }
}

/// Cross-language signing-hash fixtures. JS is the source of truth; these vectors are
/// shared with Go and spark-primitives. If this test fails, Breez's hash has drifted
/// from the rest of the ecosystem and signatures produced elsewhere will not verify.
#[cfg(test)]
mod invoice_hash_fixtures {
    use bitcoin::secp256k1::PublicKey;

    use crate::{
        Network,
        address::{SparkAddress, SparkAddressPaymentType, SparkInvoiceFields, TokensPayment},
        token::bech32m_encode_token_id,
        utils::byte_padding::BytePadding,
    };
    use base64::{Engine, engine::general_purpose::STANDARD};
    use macros::test_all;
    use platform_utils::time::{Duration, UNIX_EPOCH};
    use serde::Deserialize;

    const FIXTURE_JSON: &str =
        include_str!("../../../testdata/token_invoice_signing_hash_cases.json");

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureFile {
        test_cases: Vec<FixtureCase>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureCase {
        name: String,
        network: String,
        receiver_public_key: String,
        expected_hash: String,
        spark_invoice_fields: FixtureFields,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureFields {
        version: u32,
        id: String,
        #[serde(default)]
        memo: Option<String>,
        #[serde(default)]
        sender_public_key: Option<String>,
        #[serde(default)]
        expiry_time: Option<String>,
        #[serde(default)]
        tokens_payment: Option<FixtureTokens>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureTokens {
        #[serde(default)]
        token_identifier: Option<String>,
        #[serde(default)]
        amount: Option<String>,
    }

    fn b64(s: &str) -> Vec<u8> {
        STANDARD.decode(s).unwrap()
    }

    fn parse_network(s: &str) -> Network {
        match s {
            "MAINNET" => Network::Mainnet,
            "REGTEST" => Network::Regtest,
            "TESTNET" => Network::Testnet,
            "SIGNET" => Network::Signet,
            other => panic!("unknown network {other}"),
        }
    }

    // Hand-parse the single RFC3339 timestamp used by the fixtures. Keeps tests free of
    // a time-parsing dependency; update the match arms if new fixture cases are added.
    fn fixture_expiry_seconds(s: &str) -> u64 {
        match s {
            "2025-08-16T22:12:17.791Z" => 1_755_382_337,
            other => panic!("unhandled fixture expiry time: {other}"),
        }
    }

    fn hash_for_case(tc: &FixtureCase) -> Vec<u8> {
        let network = parse_network(&tc.network);
        let receiver_pk = PublicKey::from_slice(&b64(&tc.receiver_public_key)).unwrap();

        let id_bytes: [u8; 16] = b64(&tc.spark_invoice_fields.id).try_into().unwrap();
        let id = uuid::Uuid::from_bytes(id_bytes);

        let sender_public_key = tc
            .spark_invoice_fields
            .sender_public_key
            .as_deref()
            .map(|s| PublicKey::from_slice(&b64(s)).unwrap());

        let expiry_time = tc
            .spark_invoice_fields
            .expiry_time
            .as_deref()
            .map(|s| UNIX_EPOCH + Duration::from_secs(fixture_expiry_seconds(s)));

        let payment_type = tc.spark_invoice_fields.tokens_payment.as_ref().map(|tp| {
            let token_identifier = tp.token_identifier.as_deref().map(|s| {
                let raw = b64(s);
                assert_eq!(raw.len(), 32, "fixture token id must be 32 bytes");
                bech32m_encode_token_id(&raw, network).unwrap()
            });
            let amount = tp
                .amount
                .as_deref()
                .map(|s| u128::from_unpadded_be_bytes(&b64(s)).unwrap());
            SparkAddressPaymentType::TokensPayment(TokensPayment {
                token_identifier,
                amount,
            })
        });

        let fields = SparkInvoiceFields {
            id,
            version: tc.spark_invoice_fields.version,
            memo: tc.spark_invoice_fields.memo.clone(),
            sender_public_key,
            expiry_time,
            payment_type,
        };

        SparkAddress::new(receiver_pk, network, Some(fields))
            .compute_invoice_hash()
            .unwrap()
    }

    #[test_all]
    fn compute_invoice_hash_matches_cross_language_fixtures() {
        let fixture: FixtureFile = serde_json::from_str(FIXTURE_JSON).unwrap();
        assert!(!fixture.test_cases.is_empty(), "no fixture cases loaded");

        for tc in &fixture.test_cases {
            let hash = hash_for_case(tc);
            assert_eq!(
                hex::encode(&hash),
                tc.expected_hash,
                "fixture case `{}` hash mismatch",
                tc.name
            );
        }
    }
}
