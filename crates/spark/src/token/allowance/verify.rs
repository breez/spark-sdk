use bitcoin::secp256k1::{Message, PublicKey, Secp256k1, ecdsa, schnorr};

use crate::{
    operator::rpc::spark_token::{
        QueryTokenAllowancesRequest, RevokeTokenAllowancePayload, TokenAllowanceInfo,
        TokenAllowancePayload, TokenAllowanceStatus,
    },
    services::ServiceError,
    token::allowance::hash::{hash_create_statement, hash_revoke_statement},
};

pub(crate) fn verify_created_record(
    info: &TokenAllowanceInfo,
    signed: &TokenAllowancePayload,
) -> Result<(), ServiceError> {
    match &info.allowance_payload {
        Some(returned)
            if returned.allowance_id == signed.allowance_id
                && returned.owner_public_key == signed.owner_public_key =>
        {
            Ok(())
        }
        _ => Err(ServiceError::Generic(
            "the operator returned a different allowance than the one created".to_string(),
        )),
    }
}

pub(crate) fn verify_queried_record(
    info: &TokenAllowanceInfo,
    query: &QueryTokenAllowancesRequest,
) -> Result<(), ServiceError> {
    let payload = info
        .allowance_payload
        .as_ref()
        .ok_or_else(|| ServiceError::Generic("allowance record has no payload".to_string()))?;
    let wanted =
        |filter: Option<&Vec<u8>>, value: &[u8]| filter.is_none_or(|f| f.as_slice() == value);
    if wanted(query.owner_public_key.as_ref(), &payload.owner_public_key)
        && wanted(
            query.spender_public_key.as_ref(),
            &payload.spender_public_key,
        )
        && wanted(query.token_identifier.as_ref(), &payload.token_identifier)
    {
        return Ok(());
    }
    Err(ServiceError::Generic(
        "the operator returned an allowance the query didn't ask for".to_string(),
    ))
}

pub(crate) fn verify_allowance_record(info: &TokenAllowanceInfo) -> Result<(), ServiceError> {
    let payload = info
        .allowance_payload
        .as_ref()
        .ok_or_else(|| ServiceError::Generic("allowance record has no payload".to_string()))?;
    let owner = PublicKey::from_slice(&payload.owner_public_key)
        .map_err(|_| ServiceError::InvalidPublicKey)?;
    if !owner_signed(
        &info.owner_signature,
        &hash_create_statement(payload)?,
        &owner,
    ) {
        return Err(ServiceError::SignatureVerificationFailed(
            "owner signature does not match the allowance terms".to_string(),
        ));
    }
    if info.status() == TokenAllowanceStatus::Revoked {
        let revoke = RevokeTokenAllowancePayload {
            version: info.revoke_version,
            allowance_id: payload.allowance_id.clone(),
            owner_public_key: payload.owner_public_key.clone(),
            owner_provided_timestamp: info.owner_provided_revoke_timestamp,
        };
        if !owner_signed(
            &info.revoke_signature,
            &hash_revoke_statement(&revoke)?,
            &owner,
        ) {
            return Err(ServiceError::SignatureVerificationFailed(
                "revoke signature does not match the allowance".to_string(),
            ));
        }
    }
    Ok(())
}

fn owner_signed(signature: &[u8], hash: &[u8; 32], owner: &PublicKey) -> bool {
    let secp = Secp256k1::verification_only();
    let message = Message::from_digest(*hash);
    if signature.len() == 64
        && let Ok(sig) = schnorr::Signature::from_slice(signature)
        && secp
            .verify_schnorr(&sig, &message, &owner.x_only_public_key().0)
            .is_ok()
    {
        return true;
    }
    let Ok(mut sig) = ecdsa::Signature::from_der(signature) else {
        return false;
    };
    sig.normalize_s();
    secp.verify_ecdsa(&message, &sig, owner).is_ok()
}

#[cfg(test)]
mod tests {
    use bitcoin::secp256k1::{Keypair, Message, PublicKey, Secp256k1, SecretKey};
    use macros::test_all;
    use uuid::Uuid;

    use super::{verify_allowance_record, verify_created_record, verify_queried_record};
    use crate::{
        Network,
        operator::rpc::spark_token::{
            QueryTokenAllowancesRequest, TokenAllowanceInfo, TokenAllowancePayload,
            TokenAllowanceStatus,
        },
        token::{
            allowance::{
                hash::{hash_create_statement, hash_revoke_statement},
                model::{NewTokenAllowance, revoke_payload},
            },
            bech32m_encode_token_id,
        },
    };

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    const NOW_MS: u64 = 1_800_000_000_000;

    fn owner_secret() -> SecretKey {
        SecretKey::from_slice(&[1; 32]).unwrap()
    }

    fn record(status: TokenAllowanceStatus) -> TokenAllowanceInfo {
        let secp = Secp256k1::new();
        let owner = PublicKey::from_secret_key(&secp, &owner_secret());
        let spender = PublicKey::from_secret_key(&secp, &SecretKey::from_slice(&[2; 32]).unwrap());
        let id = Uuid::now_v7();
        let payload = NewTokenAllowance {
            spender_public_key: spender,
            token_identifier: bech32m_encode_token_id(&[7; 32], Network::Regtest).unwrap(),
            max_per_payment: Some(10),
            max_total: Some(100),
            expiry_time: NOW_MS / 1000 + 3600,
            allowed_recipients: vec![],
        }
        .to_payload(id, owner, Network::Regtest, NOW_MS)
        .unwrap();
        let keypair = Keypair::from_secret_key(&secp, &owner_secret());
        let owner_signature = secp
            .sign_schnorr_no_aux_rand(
                &Message::from_digest(hash_create_statement(&payload).unwrap()),
                &keypair,
            )
            .serialize()
            .to_vec();
        let revoke_signature = secp
            .sign_ecdsa(
                &Message::from_digest(
                    hash_revoke_statement(&revoke_payload(id, owner, NOW_MS + 1)).unwrap(),
                ),
                &owner_secret(),
            )
            .serialize_der()
            .to_vec();
        TokenAllowanceInfo {
            allowance_payload: Some(payload),
            spent_amount: vec![],
            status: status as i32,
            owner_signature,
            revoke_signature,
            owner_provided_revoke_timestamp: NOW_MS + 1,
            revoke_version: 1,
        }
    }

    #[test_all]
    fn accepts_schnorr_grant_and_der_revoke() {
        assert!(verify_allowance_record(&record(TokenAllowanceStatus::Active)).is_ok());
        assert!(verify_allowance_record(&record(TokenAllowanceStatus::Revoked)).is_ok());
    }

    #[test_all]
    fn rejects_tampered_terms() {
        let mut info = record(TokenAllowanceStatus::Active);
        info.allowance_payload.as_mut().unwrap().total_limit = 101u128.to_be_bytes().to_vec();
        assert!(verify_allowance_record(&info).is_err());
    }

    #[test_all]
    fn rejects_wrong_revoke_signature() {
        let mut info = record(TokenAllowanceStatus::Revoked);
        info.owner_provided_revoke_timestamp += 1;
        assert!(verify_allowance_record(&info).is_err());
    }

    #[test_all]
    fn accepts_only_the_created_allowance() {
        let info = record(TokenAllowanceStatus::Active);
        let signed = info.allowance_payload.clone().unwrap();
        assert!(verify_created_record(&info, &signed).is_ok());
        let other_id = TokenAllowancePayload {
            allowance_id: Uuid::now_v7().as_bytes().to_vec(),
            ..signed.clone()
        };
        assert!(verify_created_record(&info, &other_id).is_err());
        let other_owner = TokenAllowancePayload {
            owner_public_key: signed.spender_public_key.clone(),
            ..signed
        };
        assert!(verify_created_record(&info, &other_owner).is_err());
    }

    #[test_all]
    fn accepts_only_queried_records() {
        let info = record(TokenAllowanceStatus::Active);
        let payload = info.allowance_payload.clone().unwrap();
        let query = |owner: Option<Vec<u8>>, spender: Option<Vec<u8>>, token: Option<Vec<u8>>| {
            QueryTokenAllowancesRequest {
                owner_public_key: owner,
                spender_public_key: spender,
                token_identifier: token,
                ..Default::default()
            }
        };
        let owner = Some(payload.owner_public_key.clone());
        let spender = Some(payload.spender_public_key.clone());
        let token = Some(payload.token_identifier.clone());
        assert!(
            verify_queried_record(&info, &query(owner.clone(), spender.clone(), token)).is_ok()
        );
        assert!(verify_queried_record(&info, &query(None, spender.clone(), None)).is_ok());
        assert!(verify_queried_record(&info, &query(spender.clone(), None, None)).is_err());
        assert!(verify_queried_record(&info, &query(None, owner, None)).is_err());
        assert!(verify_queried_record(&info, &query(None, spender, Some(vec![8; 32]))).is_err());
    }
}
