use bitcoin::hashes::Hash as _;

use crate::{
    Network,
    operator::rpc::spark_token::{RevokeTokenAllowancePayload, TokenAllowancePayload},
    services::ServiceError,
    utils::tagged_hasher::TaggedHasher,
};

const CREATE_TAG: [&str; 4] = ["spark", "token", "create_token_allowance", "v1"];
const REVOKE_TAG: [&str; 4] = ["spark", "token", "revoke_token_allowance", "v1"];
const DELEGATED_SPEND_TAG: [&str; 4] = ["spark", "token", "delegated_spend", "v1"];

fn require_len(name: &str, value: &[u8], expected: usize) -> Result<(), ServiceError> {
    if value.len() == expected {
        return Ok(());
    }
    Err(ServiceError::InvalidInput(format!(
        "{name} must be {expected} bytes, got {}",
        value.len()
    )))
}

pub(crate) fn hash_create_statement(
    payload: &TokenAllowancePayload,
) -> Result<[u8; 32], ServiceError> {
    require_len("allowance_id", &payload.allowance_id, 16)?;
    require_len("owner_public_key", &payload.owner_public_key, 33)?;
    require_len("spender_public_key", &payload.spender_public_key, 33)?;
    require_len("token_identifier", &payload.token_identifier, 32)?;
    require_len("per_transaction_cap", &payload.per_transaction_cap, 16)?;
    require_len("total_limit", &payload.total_limit, 16)?;
    for key in &payload.recipient_allowlist {
        require_len("recipient_allowlist entry", key, 33)?;
    }
    let network = Network::from_proto_network(payload.network)
        .map_err(|e| ServiceError::InvalidInput(format!("invalid network: {e}")))?;
    let mut allowlist = payload.recipient_allowlist.clone();
    allowlist.sort();
    let expiry = payload
        .expiry_time
        .as_ref()
        .map_or(0, |t| u64::try_from(t.seconds).unwrap_or(0));

    let mut hasher = TaggedHasher::new(&CREATE_TAG)
        .add_u64(u64::from(payload.version))
        .add_string(&network.to_string())
        .add_bytes(&payload.allowance_id)
        .add_bytes(&payload.owner_public_key)
        .add_bytes(&payload.spender_public_key)
        .add_bytes(&payload.token_identifier)
        .add_bytes(&payload.per_transaction_cap)
        .add_u64(u64::from(payload.per_transaction_unlimited))
        .add_bytes(&payload.total_limit)
        .add_u64(u64::from(payload.total_unlimited))
        .add_u64(allowlist.len() as u64);
    for key in &allowlist {
        hasher = hasher.add_bytes(key);
    }
    Ok(hasher
        .add_u64(expiry)
        .add_u64(payload.owner_provided_timestamp)
        .hash()
        .to_byte_array())
}

pub(crate) fn hash_revoke_statement(
    payload: &RevokeTokenAllowancePayload,
) -> Result<[u8; 32], ServiceError> {
    require_len("allowance_id", &payload.allowance_id, 16)?;
    require_len("owner_public_key", &payload.owner_public_key, 33)?;
    Ok(TaggedHasher::new(&REVOKE_TAG)
        .add_u64(u64::from(payload.version))
        .add_bytes(&payload.allowance_id)
        .add_bytes(&payload.owner_public_key)
        .add_u64(payload.owner_provided_timestamp)
        .hash()
        .to_byte_array())
}

pub(crate) fn hash_delegated_spend(
    partial_hash: &[u8],
    allowance_id: &[u8],
) -> Result<[u8; 32], ServiceError> {
    require_len("partial_hash", partial_hash, 32)?;
    require_len("allowance_id", allowance_id, 16)?;
    Ok(TaggedHasher::new(&DELEGATED_SPEND_TAG)
        .add_bytes(partial_hash)
        .add_bytes(allowance_id)
        .hash()
        .to_byte_array())
}

#[cfg(test)]
mod tests {
    use macros::test_all;
    use prost_types::Timestamp;

    use super::{hash_create_statement, hash_delegated_spend, hash_revoke_statement};
    use crate::operator::rpc::{
        spark::Network as ProtoNetwork,
        spark_token::{RevokeTokenAllowancePayload, TokenAllowancePayload},
    };

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    const OWNER: &str = "02ca75659458529755b77663f18282f4aa130313e098fac40deffb1208207a2ffe";
    const SPENDER: &str = "033e40d72117ee89f7bda15d2b3d779843e6721e8e4c5078c192b50fb3782de2f5";
    const RECIPIENT_1: &str = "0375a9121cd7c3684ca1941978cc0dc42ce316fddf70261643f17ba3eeca6d10f2";
    const RECIPIENT_2: &str = "028c094a432d46a0ac95349d792c2e3730bd60c29188db716f56a99e39b95338b4";
    const ALLOWANCE_ID: &str = "0123456789abcdef0123456789abcdef";
    const TOKEN_ID: &str = "3e534a8d9798fe5e20516f9b1aa05f5d78d718ece893e8af89d678c3d88f2451";
    const TIMESTAMP_MS: u64 = 1_747_337_980_820;
    const EXPIRY_SECONDS: i64 = 2_000_000_000;
    const CREATE_VECTOR: &str = "df52577d7fd9feda71cdd93ba54f96f19cb4bc009ec56148e95de083f9381f58";

    fn bytes(value: &str) -> Vec<u8> {
        hex::decode(value).unwrap()
    }

    fn create_payload() -> TokenAllowancePayload {
        TokenAllowancePayload {
            version: 1,
            allowance_id: bytes(ALLOWANCE_ID),
            owner_public_key: bytes(OWNER),
            spender_public_key: bytes(SPENDER),
            token_identifier: bytes(TOKEN_ID),
            per_transaction_cap: 10_000u128.to_be_bytes().to_vec(),
            total_limit: 100_000u128.to_be_bytes().to_vec(),
            recipient_allowlist: vec![bytes(RECIPIENT_1), bytes(RECIPIENT_2)],
            expiry_time: Some(Timestamp {
                seconds: EXPIRY_SECONDS,
                nanos: 0,
            }),
            network: ProtoNetwork::Regtest as i32,
            owner_provided_timestamp: TIMESTAMP_MS,
            per_transaction_unlimited: false,
            total_unlimited: false,
        }
    }

    #[test_all]
    fn create_statement_matches_known_vector() {
        let hash = hash_create_statement(&create_payload()).unwrap();
        assert_eq!(hex::encode(hash), CREATE_VECTOR);
    }

    #[test_all]
    fn create_statement_ignores_allowlist_order() {
        let mut payload = create_payload();
        payload.recipient_allowlist.reverse();
        assert_eq!(
            hex::encode(hash_create_statement(&payload).unwrap()),
            CREATE_VECTOR
        );
    }

    #[test_all]
    fn unlimited_create_statement_matches_known_vector() {
        let mut payload = create_payload();
        payload.per_transaction_unlimited = true;
        payload.total_unlimited = true;
        payload.per_transaction_cap = vec![0; 16];
        payload.total_limit = vec![0; 16];
        assert_eq!(
            hex::encode(hash_create_statement(&payload).unwrap()),
            "373edc3c0929cf645992a07994b3cbafa6e8b5e97f847d1eca1a2491b13eec9a"
        );
    }

    #[test_all]
    fn revoke_statement_matches_known_vector() {
        let payload = RevokeTokenAllowancePayload {
            version: 1,
            allowance_id: bytes(ALLOWANCE_ID),
            owner_public_key: bytes(OWNER),
            owner_provided_timestamp: TIMESTAMP_MS,
        };
        assert_eq!(
            hex::encode(hash_revoke_statement(&payload).unwrap()),
            "e35cf0188bae34706871d27f8c87797aa00df737d63ff65191ef0e62d2afc256"
        );
    }

    #[test_all]
    fn delegated_spend_matches_known_vector() {
        let partial_hash: Vec<u8> = (1u8..=32).collect();
        let allowance_id: Vec<u8> = (0xa0u8..=0xaf).collect();
        assert_eq!(
            hex::encode(hash_delegated_spend(&partial_hash, &allowance_id).unwrap()),
            "5e7bee8c8dc242ea27c415ce43c345d7b0b7688714a75092dc1760a32f371549"
        );
    }

    #[test_all]
    fn statements_reject_wrong_lengths() {
        let mut payload = create_payload();
        payload.allowance_id = vec![0; 15];
        assert!(hash_create_statement(&payload).is_err());
        assert!(hash_delegated_spend(&[0; 31], &[0; 16]).is_err());
        assert!(hash_delegated_spend(&[0; 32], &[0; 15]).is_err());
    }
}
