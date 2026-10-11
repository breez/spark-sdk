use bitcoin::secp256k1::PublicKey;
use prost_types::Timestamp;
use uuid::Uuid;

use crate::{
    Network,
    operator::rpc::spark_token::{
        RevokeTokenAllowancePayload, TokenAllowanceInfo, TokenAllowancePayload,
        TokenAllowanceStatus as ProtoStatus,
    },
    services::ServiceError,
    token::{bech32m_decode_token_id, bech32m_encode_token_id},
    utils::byte_padding::BytePadding,
};

const STATEMENT_VERSION: u32 = 1;
const MAX_ALLOWED_RECIPIENTS: usize = 256;
const MAX_EXPIRY_SECONDS: u64 = 100_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenAllowanceStatus {
    Active,
    Exhausted,
    Expired,
    Revoked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenAllowanceRole {
    Owner,
    Spender,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenAllowance {
    pub id: String,
    pub owner_public_key: PublicKey,
    pub spender_public_key: PublicKey,
    pub token_identifier: String,
    pub max_per_payment: Option<u128>,
    pub max_total: Option<u128>,
    pub spent_amount: u128,
    pub allowed_recipients: Vec<PublicKey>,
    pub expiry_time: u64,
    pub created_at: u64,
    pub revoked_at: Option<u64>,
    pub status: TokenAllowanceStatus,
}

#[derive(Clone, Debug)]
pub struct NewTokenAllowance {
    pub spender_public_key: PublicKey,
    pub token_identifier: String,
    pub max_per_payment: Option<u128>,
    pub max_total: Option<u128>,
    pub expiry_time: u64,
    pub allowed_recipients: Vec<PublicKey>,
}

#[derive(Clone, Debug)]
pub struct TokenAllowanceQuery {
    pub role: TokenAllowanceRole,
    pub counterparty_public_key: Option<PublicKey>,
    pub token_identifier: Option<String>,
    pub include_inactive: bool,
    pub offset: u32,
    pub limit: u32,
}

fn invalid(message: &str) -> ServiceError {
    ServiceError::InvalidInput(message.to_string())
}

fn encode_limit(limit: Option<u128>) -> (Vec<u8>, bool) {
    match limit {
        Some(value) => (value.to_be_bytes().to_vec(), false),
        None => (vec![0; 16], true),
    }
}

fn decode_limit(bytes: &[u8], unlimited: bool) -> Result<Option<u128>, ServiceError> {
    if unlimited {
        return Ok(None);
    }
    let array: [u8; 16] = bytes
        .try_into()
        .map_err(|_| ServiceError::Generic("allowance limit must be 16 bytes".to_string()))?;
    Ok(Some(u128::from_be_bytes(array)))
}

fn parse_key(bytes: &[u8]) -> Result<PublicKey, ServiceError> {
    PublicKey::from_slice(bytes).map_err(|_| ServiceError::InvalidPublicKey)
}

impl NewTokenAllowance {
    pub(crate) fn to_payload(
        &self,
        allowance_id: Uuid,
        owner: PublicKey,
        network: Network,
        now_ms: u64,
    ) -> Result<TokenAllowancePayload, ServiceError> {
        if self.spender_public_key == owner {
            return Err(invalid("the spender must differ from the owner"));
        }
        if self.max_per_payment == Some(0) || self.max_total == Some(0) {
            return Err(invalid("allowance limits must be positive"));
        }
        if let (Some(per_payment), Some(total)) = (self.max_per_payment, self.max_total)
            && per_payment > total
        {
            return Err(invalid("max_per_payment must not exceed max_total"));
        }
        if self.expiry_time >= MAX_EXPIRY_SECONDS {
            return Err(invalid("expiry_time must be in Unix seconds"));
        }
        if self.expiry_time <= now_ms / 1000 {
            return Err(invalid("expiry_time must be in the future"));
        }
        if self.allowed_recipients.len() > MAX_ALLOWED_RECIPIENTS {
            return Err(invalid("at most 256 allowed recipients"));
        }
        if self.allowed_recipients.contains(&owner) {
            return Err(invalid("allowed recipients must not include the owner"));
        }
        let token_identifier = bech32m_decode_token_id(&self.token_identifier, Some(network))?;
        let (per_transaction_cap, per_transaction_unlimited) = encode_limit(self.max_per_payment);
        let (total_limit, total_unlimited) = encode_limit(self.max_total);
        let expiry_seconds =
            i64::try_from(self.expiry_time).map_err(|_| invalid("expiry_time is too large"))?;
        Ok(TokenAllowancePayload {
            version: STATEMENT_VERSION,
            allowance_id: allowance_id.as_bytes().to_vec(),
            owner_public_key: owner.serialize().to_vec(),
            spender_public_key: self.spender_public_key.serialize().to_vec(),
            token_identifier,
            per_transaction_cap,
            total_limit,
            recipient_allowlist: self
                .allowed_recipients
                .iter()
                .map(|k| k.serialize().to_vec())
                .collect(),
            expiry_time: Some(Timestamp {
                seconds: expiry_seconds,
                nanos: 0,
            }),
            network: network.to_proto_network() as i32,
            owner_provided_timestamp: now_ms,
            per_transaction_unlimited,
            total_unlimited,
        })
    }
}

pub(crate) fn revoke_payload(
    allowance_id: Uuid,
    owner: PublicKey,
    now_ms: u64,
) -> RevokeTokenAllowancePayload {
    RevokeTokenAllowancePayload {
        version: STATEMENT_VERSION,
        allowance_id: allowance_id.as_bytes().to_vec(),
        owner_public_key: owner.serialize().to_vec(),
        owner_provided_timestamp: now_ms,
    }
}

impl TokenAllowance {
    pub(crate) fn from_info(
        info: &TokenAllowanceInfo,
        network: Network,
        now_secs: u64,
    ) -> Result<Self, ServiceError> {
        let payload = info
            .allowance_payload
            .as_ref()
            .ok_or_else(|| ServiceError::Generic("allowance record has no payload".to_string()))?;
        let expiry_time = payload
            .expiry_time
            .as_ref()
            .map_or(0, |t| u64::try_from(t.seconds).unwrap_or(0));
        let status = match info.status() {
            ProtoStatus::Active | ProtoStatus::Exhausted if expiry_time <= now_secs => {
                TokenAllowanceStatus::Expired
            }
            ProtoStatus::Active => TokenAllowanceStatus::Active,
            ProtoStatus::Exhausted => TokenAllowanceStatus::Exhausted,
            ProtoStatus::Expired => TokenAllowanceStatus::Expired,
            ProtoStatus::Revoked => TokenAllowanceStatus::Revoked,
            ProtoStatus::Unspecified => {
                return Err(ServiceError::Generic(
                    "allowance record has no status".to_string(),
                ));
            }
        };
        let id = Uuid::from_slice(&payload.allowance_id)
            .map_err(|e| ServiceError::Generic(format!("invalid allowance id: {e}")))?;
        let spent_amount = u128::from_unpadded_be_bytes(&info.spent_amount)
            .map_err(|e| ServiceError::Generic(format!("invalid spent amount: {e}")))?;
        Ok(Self {
            id: id.to_string(),
            owner_public_key: parse_key(&payload.owner_public_key)?,
            spender_public_key: parse_key(&payload.spender_public_key)?,
            token_identifier: bech32m_encode_token_id(&payload.token_identifier, network)?,
            max_per_payment: decode_limit(
                &payload.per_transaction_cap,
                payload.per_transaction_unlimited,
            )?,
            max_total: decode_limit(&payload.total_limit, payload.total_unlimited)?,
            spent_amount,
            allowed_recipients: payload
                .recipient_allowlist
                .iter()
                .map(|k| parse_key(k))
                .collect::<Result<_, _>>()?,
            expiry_time,
            created_at: payload.owner_provided_timestamp / 1000,
            revoked_at: (status == TokenAllowanceStatus::Revoked)
                .then_some(info.owner_provided_revoke_timestamp / 1000),
            status,
        })
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
    use macros::test_all;
    use uuid::Uuid;

    use super::{NewTokenAllowance, TokenAllowance, TokenAllowanceStatus};
    use crate::{
        Network,
        operator::rpc::spark_token::{TokenAllowanceInfo, TokenAllowanceStatus as Proto},
        token::bech32m_encode_token_id,
    };

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    const NOW_MS: u64 = 1_800_000_000_000;

    fn key(fill: u8) -> PublicKey {
        PublicKey::from_secret_key(
            &Secp256k1::new(),
            &SecretKey::from_slice(&[fill; 32]).unwrap(),
        )
    }

    fn request() -> NewTokenAllowance {
        NewTokenAllowance {
            spender_public_key: key(2),
            token_identifier: bech32m_encode_token_id(&[7; 32], Network::Regtest).unwrap(),
            max_per_payment: Some(5_000),
            max_total: None,
            expiry_time: NOW_MS / 1000 + 3600,
            allowed_recipients: vec![key(3)],
        }
    }

    #[test_all]
    fn payload_encodes_limits_and_flags() {
        let payload = request()
            .to_payload(Uuid::now_v7(), key(1), Network::Regtest, NOW_MS)
            .unwrap();
        assert_eq!(
            payload.per_transaction_cap,
            5_000u128.to_be_bytes().to_vec()
        );
        assert!(!payload.per_transaction_unlimited);
        assert_eq!(payload.total_limit, vec![0; 16]);
        assert!(payload.total_unlimited);
        assert_eq!(payload.owner_provided_timestamp, NOW_MS);
        assert_eq!(
            payload.recipient_allowlist,
            vec![key(3).serialize().to_vec()]
        );
    }

    #[test_all]
    fn payload_rejects_invalid_requests() {
        let owner = key(1);
        let id = Uuid::now_v7();
        let reject = |request: NewTokenAllowance| {
            assert!(
                request
                    .to_payload(id, owner, Network::Regtest, NOW_MS)
                    .is_err()
            );
        };
        reject(NewTokenAllowance {
            spender_public_key: owner,
            ..request()
        });
        reject(NewTokenAllowance {
            max_per_payment: Some(0),
            ..request()
        });
        reject(NewTokenAllowance {
            max_per_payment: Some(10),
            max_total: Some(5),
            ..request()
        });
        reject(NewTokenAllowance {
            expiry_time: NOW_MS / 1000,
            ..request()
        });
        reject(NewTokenAllowance {
            expiry_time: NOW_MS + 3_600_000,
            ..request()
        });
        reject(NewTokenAllowance {
            allowed_recipients: vec![owner],
            ..request()
        });
        reject(NewTokenAllowance {
            allowed_recipients: vec![key(3); 257],
            ..request()
        });
        reject(NewTokenAllowance {
            token_identifier: bech32m_encode_token_id(&[7; 32], Network::Mainnet).unwrap(),
            ..request()
        });
    }

    #[test_all]
    fn record_converts_to_allowance() {
        let id = Uuid::now_v7();
        let payload = request()
            .to_payload(id, key(1), Network::Regtest, NOW_MS)
            .unwrap();
        let info = TokenAllowanceInfo {
            allowance_payload: Some(payload),
            spent_amount: vec![0x03, 0xe8],
            status: Proto::Revoked as i32,
            owner_signature: vec![],
            revoke_signature: vec![],
            owner_provided_revoke_timestamp: NOW_MS + 5_000,
            revoke_version: 1,
        };
        let allowance = TokenAllowance::from_info(&info, Network::Regtest, NOW_MS / 1000).unwrap();
        assert_eq!(allowance.id, id.to_string());
        assert_eq!(allowance.owner_public_key, key(1));
        assert_eq!(allowance.max_per_payment, Some(5_000));
        assert_eq!(allowance.max_total, None);
        assert_eq!(allowance.spent_amount, 1_000);
        assert_eq!(allowance.created_at, NOW_MS / 1000);
        assert_eq!(allowance.revoked_at, Some((NOW_MS + 5_000) / 1000));
        assert_eq!(allowance.status, TokenAllowanceStatus::Revoked);
    }

    #[test_all]
    fn a_record_past_expiry_lists_as_expired() {
        let payload = request()
            .to_payload(Uuid::now_v7(), key(1), Network::Regtest, NOW_MS)
            .unwrap();
        let expiry = request().expiry_time;
        let listed = |status: Proto, now_secs: u64| {
            let info = TokenAllowanceInfo {
                allowance_payload: Some(payload.clone()),
                status: status as i32,
                ..Default::default()
            };
            TokenAllowance::from_info(&info, Network::Regtest, now_secs)
                .unwrap()
                .status
        };
        assert_eq!(
            listed(Proto::Active, expiry - 1),
            TokenAllowanceStatus::Active
        );
        assert_eq!(listed(Proto::Active, expiry), TokenAllowanceStatus::Expired);
        assert_eq!(
            listed(Proto::Exhausted, expiry - 1),
            TokenAllowanceStatus::Exhausted
        );
        assert_eq!(
            listed(Proto::Exhausted, expiry + 1),
            TokenAllowanceStatus::Expired
        );
        assert_eq!(
            listed(Proto::Revoked, expiry + 1),
            TokenAllowanceStatus::Revoked
        );
    }
}
