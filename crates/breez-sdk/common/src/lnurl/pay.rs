use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use base64::{Engine, prelude::BASE64_STANDARD};
use serde::{Deserialize, Serialize};

use crate::{
    ensure_sdk,
    input::parse_invoice,
    invoice::{InvoiceError, validate_network},
    lnurl::{
        LnurlErrorDetails,
        error::{LnurlError, LnurlResult},
    },
    network::BitcoinNetwork,
    utils::default_true,
};

use platform_utils::HttpClient;

pub type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
pub type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

/// Validates invoice and performs the second and last step of LNURL-pay, as per
/// <https://github.com/lnurl/luds/blob/luds/06.md>
///
/// See the [parse] docs for more detail on the full workflow.
pub async fn validate_lnurl_pay<C: HttpClient + ?Sized>(
    http_client: &C,
    user_amount_msat: u64,
    comment: &Option<String>,
    pay_request: &LnurlPayRequestDetails,
    network: BitcoinNetwork,
    validate_success_action_url: Option<bool>,
) -> LnurlResult<ValidatedCallbackResponse> {
    validate_user_input(
        user_amount_msat,
        comment,
        pay_request.min_sendable,
        pay_request.max_sendable,
        pay_request.comment_allowed,
    )?;

    let callback_url = build_pay_callback_url(user_amount_msat, comment, pay_request)?;
    let response = http_client.get(callback_url, None).await?;
    if let Ok(err) = response.json::<LnurlErrorDetails>() {
        return Ok(ValidatedCallbackResponse::EndpointError { data: err });
    }

    let mut callback_resp: CallbackResponse = response
        .json()
        .map_err(|e| LnurlError::InvalidResponse(e.to_string()))?;
    if let Some(ref sa) = callback_resp.success_action {
        match sa {
            SuccessAction::Aes { data } => data.validate()?,
            SuccessAction::Message { data } => data.validate()?,
            SuccessAction::Url { data } => {
                callback_resp.success_action = Some(SuccessAction::Url {
                    data: data
                        .validate(pay_request, validate_success_action_url.unwrap_or(true))?,
                });
            }
        }
    }

    validate_invoice(user_amount_msat, &callback_resp.pr, network)?;
    Ok(ValidatedCallbackResponse::EndpointSuccess {
        data: callback_resp,
    })
}

pub fn build_pay_callback_url(
    user_amount_msat: u64,
    user_comment: &Option<String>,
    pay_request: &LnurlPayRequestDetails,
) -> LnurlResult<String> {
    let amount_msat = user_amount_msat.to_string();
    let mut url = url::Url::parse(&pay_request.callback)
        .map_err(|_| LnurlError::invalid_uri("invalid callback uri"))?;

    url.query_pairs_mut().append_pair("amount", &amount_msat);
    if let Some(comment) = user_comment {
        url.query_pairs_mut().append_pair("comment", comment);
    }

    Ok(url.to_string())
}

pub fn validate_user_input(
    user_amount_msat: u64,
    comment: &Option<String>,
    condition_min_amount_msat: u64,
    condition_max_amount_msat: u64,
    condition_max_comment_len: u16,
) -> LnurlResult<()> {
    ensure_sdk!(
        user_amount_msat >= condition_min_amount_msat,
        LnurlError::general("Amount is smaller than the minimum allowed")
    );

    ensure_sdk!(
        user_amount_msat <= condition_max_amount_msat,
        LnurlError::general("Amount is bigger than the maximum allowed")
    );

    let Some(comment) = comment else {
        return Ok(());
    };

    if comment.len() > condition_max_comment_len as usize {
        return Err(LnurlError::general(
            "Comment is longer than the maximum allowed comment length",
        ));
    }

    Ok(())
}

pub fn validate_invoice(
    user_amount_msat: u64,
    bolt11: &str,
    network: BitcoinNetwork,
) -> LnurlResult<()> {
    let invoice = parse_invoice(bolt11).ok_or(InvoiceError::general("invalid invoice"))?;
    // Valid the invoice network against the config network
    validate_network(&invoice, network)?;

    let Some(invoice_amount_msat) = invoice.amount_msat else {
        return Err(LnurlError::general(
            "Amount is bigger than the maximum allowed",
        ));
    };

    if invoice_amount_msat != user_amount_msat {
        return Err(LnurlError::general(
            "Invoice amount is different than the user chosen amount",
        ));
    }

    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LnurlPayRequestDetails {
    pub callback: String,
    /// The minimum amount, in millisats, that this LNURL-pay endpoint accepts
    pub min_sendable: u64,
    /// The maximum amount, in millisats, that this LNURL-pay endpoint accepts
    pub max_sendable: u64,
    /// As per LUD-06, `metadata` is a raw string (e.g. a json representation of the inner map).
    /// Use `metadata_vec()` to get the parsed items.
    #[serde(rename(deserialize = "metadata"))]
    pub metadata_str: String,
    /// The comment length accepted by this endpoint
    ///
    /// See <https://github.com/lnurl/luds/blob/luds/12.md>
    #[serde(default)]
    pub comment_allowed: u16,

    /// Indicates the domain of the LNURL-pay service, to be shown to the user when asking for
    /// payment input, as per LUD-06 spec.
    ///
    /// Note: this is not the domain of the callback, but the domain of the LNURL-pay endpoint.
    #[serde(skip)]
    pub domain: String,

    #[serde(skip)]
    pub url: String,

    /// Optional lightning address if that was used to resolve the lnurl.
    #[serde(skip)]
    pub address: Option<String>,

    /// Value indicating whether the recipient supports Nostr Zaps through NIP-57.
    ///
    /// See <https://github.com/nostr-protocol/nips/blob/master/57.md>
    pub allows_nostr: Option<bool>,
    /// Optional recipient's lnurl provider's Nostr pubkey for NIP-57. If it exists it should be a
    /// valid BIP 340 public key in hex.
    ///
    /// See <https://github.com/nostr-protocol/nips/blob/master/57.md>
    /// See <https://github.com/bitcoin/bips/blob/master/bip-0340.mediawiki>
    pub nostr_pubkey: Option<String>,
}

pub enum ValidatedCallbackResponse {
    EndpointSuccess { data: CallbackResponse },
    EndpointError { data: LnurlErrorDetails },
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CallbackResponse {
    pub pr: String,
    pub success_action: Option<SuccessAction>,
}

/// Supported success action types
///
/// Receiving any other (unsupported) success action type will result in a failed parsing,
/// which will abort the LNURL-pay workflow, as per LUD-09.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "tag")]
pub enum SuccessAction {
    /// AES type, described in LUD-10
    Aes {
        #[serde(flatten)]
        data: AesSuccessActionData,
    },

    /// Message type, described in LUD-09
    Message {
        #[serde(flatten)]
        data: MessageSuccessActionData,
    },

    /// URL type, described in LUD-09
    Url {
        #[serde(flatten)]
        data: UrlSuccessActionData,
    },
}

/// [`SuccessAction`] where contents are ready to be consumed by the caller
///
/// Contents are identical to [`SuccessAction`], except for AES where the ciphertext is decrypted.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SuccessActionProcessed {
    /// See [`SuccessAction::Aes`] for received payload
    ///
    /// See [`AesSuccessActionDataDecrypted`] for decrypted payload
    Aes { result: AesSuccessActionDataResult },

    /// See [`SuccessAction::Message`]
    Message { data: MessageSuccessActionData },

    /// See [`SuccessAction::Url`]
    Url { data: UrlSuccessActionData },
}

/// Payload of the AES success action, as received from the LNURL endpoint
///
/// See [`AesSuccessActionDataDecrypted`] for a similar wrapper containing the decrypted payload
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AesSuccessActionData {
    /// Contents description, up to 144 characters
    pub description: String,

    /// Base64, AES-encrypted data where encryption key is payment preimage, up to 4kb of characters
    pub ciphertext: String,

    /// Base64, initialization vector, exactly 24 characters
    pub iv: String,
}

impl AesSuccessActionData {
    /// Decrypts the ciphertext as a UTF-8 string, given the key (invoice preimage) parameter.
    pub fn decrypt(&self, key: &[u8; 32]) -> anyhow::Result<String> {
        let plaintext_bytes =
            Aes256CbcDec::new_from_slices(key, &BASE64_STANDARD.decode(&self.iv)?)?
                .decrypt_padded_vec_mut::<Pkcs7>(&BASE64_STANDARD.decode(&self.ciphertext)?)?;

        Ok(String::from_utf8(plaintext_bytes)?)
    }

    /// Helper method that encrypts a given plaintext, with a given key and IV.
    pub fn encrypt(key: &[u8; 32], iv: &[u8; 16], plaintext: &str) -> anyhow::Result<String> {
        let ciphertext_bytes = Aes256CbcEnc::new_from_slices(key, iv)?
            .encrypt_padded_vec_mut::<Pkcs7>(plaintext.as_bytes());

        Ok(BASE64_STANDARD.encode(ciphertext_bytes))
    }

    /// Validates the fields, but does not decrypt and validate the ciphertext.
    pub fn validate(&self) -> LnurlResult<()> {
        ensure_sdk!(
            self.description.len() <= 144,
            LnurlError::general("AES action description length is larger than the maximum allowed")
        );

        ensure_sdk!(
            self.ciphertext.len() <= 4096,
            LnurlError::general("AES action ciphertext length is larger than the maximum allowed")
        );

        BASE64_STANDARD.decode(&self.ciphertext)?;

        ensure_sdk!(
            self.iv.len() == 24,
            LnurlError::general("AES action iv has unexpected length")
        );

        BASE64_STANDARD.decode(&self.iv)?;

        Ok(())
    }
}

/// Result of decryption of [`AesSuccessActionData`] payload
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AesSuccessActionDataResult {
    Decrypted { data: AesSuccessActionDataDecrypted },
    ErrorStatus { reason: String },
}

/// Wrapper for the decrypted [`AesSuccessActionData`] payload
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AesSuccessActionDataDecrypted {
    /// Contents description, up to 144 characters
    pub description: String,

    /// Decrypted content
    pub plaintext: String,
}

impl TryFrom<(&AesSuccessActionData, &[u8; 32])> for AesSuccessActionDataDecrypted {
    type Error = anyhow::Error;

    fn try_from(
        value: (&AesSuccessActionData, &[u8; 32]),
    ) -> std::result::Result<Self, Self::Error> {
        let data = value.0;
        let key = value.1;

        Ok(AesSuccessActionDataDecrypted {
            description: data.description.clone(),
            plaintext: data.decrypt(key)?,
        })
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Deserialize, Serialize)]
pub struct MessageSuccessActionData {
    pub message: String,
}

impl MessageSuccessActionData {
    pub fn validate(&self) -> LnurlResult<()> {
        ensure_sdk!(
            self.message.len() <= 144,
            LnurlError::general("Success action message is longer than the maximum allowed length",)
        );

        Ok(())
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Deserialize, Serialize)]
pub struct UrlSuccessActionData {
    /// Contents description, up to 144 characters
    pub description: String,

    /// URL of the success action
    pub url: String,

    /// Indicates the success URL domain matches the LNURL callback domain.
    ///
    /// See <https://github.com/lnurl/luds/blob/luds/09.md>
    #[serde(default = "default_true")]
    pub matches_callback_domain: bool,
}

impl UrlSuccessActionData {
    pub fn validate(
        &self,
        pay_request: &LnurlPayRequestDetails,
        validate_url: bool,
    ) -> LnurlResult<UrlSuccessActionData> {
        let mut validated_data = self.clone();
        ensure_sdk!(
            self.description.len() <= 144,
            LnurlError::general(
                "Success action description is longer than the maximum allowed length",
            )
        );

        let req_url = url::Url::parse(&pay_request.callback)
            .map_err(|e| LnurlError::InvalidUri(e.to_string()))?;
        let req_domain = req_url
            .domain()
            .ok_or_else(|| LnurlError::InvalidUri("Could not determine callback domain".into()))?;

        let action_res_url =
            url::Url::parse(&self.url).map_err(|e| LnurlError::InvalidUri(e.to_string()))?;
        let action_res_domain = action_res_url.domain().ok_or_else(|| {
            LnurlError::invalid_uri("Could not determine Success Action URL domain")
        })?;

        if validate_url && req_domain != action_res_domain {
            return Err(LnurlError::general(
                "Success Action URL has different domain than the callback domain",
            ));
        }

        validated_data.matches_callback_domain = req_domain == action_res_domain;
        Ok(validated_data)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
    use anyhow::Result;
    use bitcoin::hashes::{Hash, sha256};

    use crate::lnurl::tests::rand_string;

    use super::*;

    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn get_test_pay_req_data(
        min_sendable: u64,
        max_sendable: u64,
        comment_len: u16,
    ) -> LnurlPayRequestDetails {
        LnurlPayRequestDetails {
            min_sendable,
            max_sendable,
            comment_allowed: comment_len,
            metadata_str: String::new(),
            callback: "http://localhost:8080/callback".into(),
            domain: "localhost".into(),
            allows_nostr: Some(false),
            nostr_pubkey: None,
            url: "http://localhost:8080/pay".into(),
            address: None,
        }
    }

    #[macros::test_all]
    fn test_lnurl_pay_validate_input() {
        assert!(validate_user_input(100_000, &None, 0, 100_000, 0).is_ok());
        assert!(validate_user_input(100_000, &Some("test".into()), 0, 100_000, 5).is_ok());

        assert!(validate_user_input(5000, &None, 10_000, 100_000, 5).is_err());
        assert!(validate_user_input(200_000, &None, 10_000, 100_000, 5).is_err());
        assert!(validate_user_input(100_000, &Some("test".into()), 10_000, 100_000, 0).is_err());
    }

    #[macros::test_all]
    fn test_lnurl_pay_success_action_deserialize() -> Result<()> {
        let aes_json_str = r#"{"tag":"aes","description":"short msg","ciphertext":"kSOatdlDaaGEdO5YNyx9D87l4ieQP2cb/hnvMvHK2oBNEPDwBiZSidk2MXND28DK","iv":"1234567890abcdef"}"#;
        let aes_deserialized_sa: SuccessAction = serde_json::from_str(aes_json_str)?;
        let aes_serialized_sa = serde_json::to_string(&aes_deserialized_sa)?;
        assert_eq!(aes_json_str, aes_serialized_sa);

        let message_json_str = r#"{"tag":"message","message":"Test message"}"#;
        let message_deserialized_sa: SuccessAction = serde_json::from_str(message_json_str)?;
        let message_serialized_sa = serde_json::to_string(&message_deserialized_sa)?;
        assert_eq!(message_json_str, message_serialized_sa);

        let url_json_str = r#"{"tag":"url","description":"short msg","url":"https://new-domain.com/test-url","matches_callback_domain":true}"#;
        let url_deserialized_sa: SuccessAction = serde_json::from_str(url_json_str)?;
        let url_serialized_sa = serde_json::to_string(&url_deserialized_sa)?;
        assert_eq!(url_json_str, url_serialized_sa);

        Ok(())
    }

    #[macros::test_all]
    fn test_lnurl_pay_validate_success_action_encrypt_decrypt() -> Result<()> {
        // Simulate a preimage, which will be the AES key
        let key = sha256::Hash::hash(&[0x42; 16]);
        let key_bytes = key.as_byte_array();

        let iv_bytes = [0x24; 16]; // 16 bytes = 24 chars
        let iv_base64 = BASE64_STANDARD.encode(iv_bytes); // JCQkJCQkJCQkJCQkJCQkJA==

        let plaintext = "hello world! this is my plaintext.";
        let plaintext_bytes = plaintext.as_bytes();

        // hex = 91239ab5d94369a18474ee58372c7d0fcee5e227903f671bfe19ef32f1cada804d10f0f006265289d936317343dbc0ca
        // base64 = kSOatdlDaaGEdO5YNyx9D87l4ieQP2cb/hnvMvHK2oBNEPDwBiZSidk2MXND28DK
        let ciphertext_bytes = &hex::decode(
            "91239ab5d94369a18474ee58372c7d0fcee5e227903f671bfe19ef32f1cada804d10f0f006265289d936317343dbc0ca",
        )?;
        let ciphertext_base64 = BASE64_STANDARD.encode(ciphertext_bytes);

        // Encrypt raw (which returns raw bytes)
        let res = Aes256CbcEnc::new_from_slices(key_bytes, &iv_bytes)?
            .encrypt_padded_vec_mut::<Pkcs7>(plaintext_bytes);
        assert_eq!(res[..], ciphertext_bytes[..]);

        // Decrypt raw (which returns raw bytes)
        let res = Aes256CbcDec::new_from_slices(key_bytes, &iv_bytes)?
            .decrypt_padded_vec_mut::<Pkcs7>(&res)?;
        assert_eq!(res[..], plaintext_bytes[..]);

        // Encrypt via AesSuccessActionData helper method (which returns a base64 representation of the bytes)
        let res = AesSuccessActionData::encrypt(key_bytes, &iv_bytes, plaintext)?;
        assert_eq!(res, BASE64_STANDARD.encode(ciphertext_bytes));

        // Decrypt via AesSuccessActionData instance method (which returns an UTF-8 string of the plaintext bytes)
        let res = AesSuccessActionData {
            description: "Test AES successData description".into(),
            ciphertext: ciphertext_base64,
            iv: iv_base64,
        }
        .decrypt(key_bytes)?;
        assert_eq!(res.as_bytes(), plaintext_bytes);

        Ok(())
    }

    #[macros::test_all]
    fn test_lnurl_pay_validate_success_action_aes() {
        assert!(
            AesSuccessActionData {
                description: "Test AES successData description".into(),
                ciphertext: "kSOatdlDaaGEdO5YNyx9D87l4ieQP2cb/hnvMvHK2oBNEPDwBiZSidk2MXND28DK"
                    .into(),
                iv: BASE64_STANDARD.encode([0xa; 16])
            }
            .validate()
            .is_ok()
        );

        // Description longer than 144 chars
        assert!(
            AesSuccessActionData {
                description: rand_string(150),
                ciphertext: "kSOatdlDaaGEdO5YNyx9D87l4ieQP2cb/hnvMvHK2oBNEPDwBiZSidk2MXND28DK"
                    .into(),
                iv: BASE64_STANDARD.encode([0xa; 16])
            }
            .validate()
            .is_err()
        );

        // IV size below 16 bytes (24 chars)
        assert!(
            AesSuccessActionData {
                description: "Test AES successData description".into(),
                ciphertext: "kSOatdlDaaGEdO5YNyx9D87l4ieQP2cb/hnvMvHK2oBNEPDwBiZSidk2MXND28DK"
                    .into(),
                iv: BASE64_STANDARD.encode([0xa; 10])
            }
            .validate()
            .is_err()
        );

        // IV size above 16 bytes (24 chars)
        assert!(
            AesSuccessActionData {
                description: "Test AES successData description".into(),
                ciphertext: "kSOatdlDaaGEdO5YNyx9D87l4ieQP2cb/hnvMvHK2oBNEPDwBiZSidk2MXND28DK"
                    .into(),
                iv: BASE64_STANDARD.encode([0xa; 20])
            }
            .validate()
            .is_err()
        );

        // IV is not base64 encoded (but fits length of 24 chars)
        assert!(
            AesSuccessActionData {
                description: "Test AES successData description".into(),
                ciphertext: "kSOatdlDaaGEdO5YNyx9D87l4ieQP2cb/hnvMvHK2oBNEPDwBiZSidk2MXND28DK"
                    .into(),
                iv: ",".repeat(24)
            }
            .validate()
            .is_err()
        );

        // Ciphertext is not base64 encoded
        assert!(
            AesSuccessActionData {
                description: "Test AES successData description".into(),
                ciphertext: ",".repeat(96),
                iv: BASE64_STANDARD.encode([0xa; 16])
            }
            .validate()
            .is_err()
        );

        // Ciphertext longer than 4KB
        assert!(
            AesSuccessActionData {
                description: "Test AES successData description".into(),
                ciphertext: BASE64_STANDARD.encode(rand_string(5000)),
                iv: BASE64_STANDARD.encode([0xa; 16])
            }
            .validate()
            .is_err()
        );
    }

    #[macros::test_all]
    fn test_lnurl_pay_validate_success_action_msg() {
        assert!(
            MessageSuccessActionData {
                message: "short msg".into()
            }
            .validate()
            .is_ok()
        );

        // Too long message
        assert!(
            MessageSuccessActionData {
                message: rand_string(150)
            }
            .validate()
            .is_err()
        );
    }

    #[macros::test_all]
    fn test_lnurl_pay_validate_success_url() {
        let pay_req_data = get_test_pay_req_data(0, 100_000, 100);

        let validated_data1 = UrlSuccessActionData {
            description: "short msg".into(),
            url: pay_req_data.callback.clone(),
            matches_callback_domain: true,
        }
        .validate(&pay_req_data, true);
        assert!(validated_data1.is_ok());
        assert!(validated_data1.unwrap().matches_callback_domain);

        // Different Success Action domain than in the callback URL with validation
        assert!(
            UrlSuccessActionData {
                description: "short msg".into(),
                url: "https://new-domain.com/test-url".into(),
                matches_callback_domain: true,
            }
            .validate(&pay_req_data, true)
            .is_err()
        );

        // Different Success Action domain than in the callback URL without validation
        let validated_data2 = UrlSuccessActionData {
            description: "short msg".into(),
            url: "https://new-domain.com/test-url".into(),
            matches_callback_domain: true,
        }
        .validate(&pay_req_data, false);
        assert!(validated_data2.is_ok());
        assert!(!validated_data2.unwrap().matches_callback_domain);

        // Too long description
        assert!(
            UrlSuccessActionData {
                description: rand_string(150),
                url: pay_req_data.callback.clone(),
                matches_callback_domain: true,
            }
            .validate(&pay_req_data, true)
            .is_err()
        );
    }
}
