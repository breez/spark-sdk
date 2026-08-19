use std::str::FromStr;

use bitcoin::hex::DisplayHex;
use lnurl_models::{sanitize_username, signed_message};
use platform_utils::time::{SystemTime, UNIX_EPOCH};

use crate::{
    AuthorizeTransferRequest, CheckLightningAddressRequest, ClaimTransferRequest,
    LightningAddressInfo, LnurlInfo, RegisterLightningAddressRequest, TransferAuthorization,
    error::SdkError, lnurl::LnurlServerError, persist::ObjectCacheRepository,
};

use super::BreezSdk;

fn now_secs() -> Result<u64, SdkError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|_| SdkError::Generic("system clock is before the Unix epoch".to_string()))
}

/// Lowercase compressed hex, the form the server rebuilds the signed message
/// with. A caller-supplied pubkey that differs only in case or encoding would
/// otherwise produce a message the server never reconstructs, and the only
/// symptom would be "invalid signature".
fn normalized_pubkey(pubkey: &str) -> Result<String, SdkError> {
    bitcoin::secp256k1::PublicKey::from_str(pubkey)
        .map(|pubkey| pubkey.to_string())
        .map_err(|_| SdkError::InvalidInput(format!("'{pubkey}' is not a valid public key")))
}

/// The domain a `{username}@{domain}` lightning address lives on.
///
/// The address is the server's own record of the domain it resolved when the
/// registration was made, so it names where the registration lives even after
/// this SDK is pointed elsewhere.
fn address_domain(lightning_address: &str) -> Result<String, SdkError> {
    lightning_address
        .rsplit_once('@')
        .map(|(_, domain)| domain.to_ascii_lowercase())
        .ok_or_else(|| {
            SdkError::Generic(format!(
                "cached lightning address '{lightning_address}' has no domain"
            ))
        })
}

/// Rejects a domain that is not the one this SDK talks to, naming both so the
/// failure says which server that is.
///
/// This belongs to the side that makes the request: the domain its server
/// resolves is what a signature has to match. The side producing a signature for
/// someone else to submit has no such constraint.
fn require_configured_domain(
    configured_domain: &str,
    domain: &str,
    subject: &str,
) -> Result<(), SdkError> {
    let configured = crate::lnurl::signed_domain(configured_domain);
    if domain != configured {
        return Err(SdkError::InvalidInput(format!(
            "{subject} names domain '{domain}', but this SDK is configured for '{configured}'"
        )));
    }
    Ok(())
}

/// Names the address as a payer would type it, so the configured domain is
/// reduced to the authority the server registers the address under rather than
/// interpolated whole.
fn default_description(username: &str, configured_domain: &str) -> String {
    format!(
        "Pay to {username}@{}",
        crate::lnurl::signed_domain(configured_domain)
    )
}

#[cfg_attr(feature = "uniffi", uniffi::export(async_runtime = "tokio"))]
#[allow(clippy::needless_pass_by_value)]
impl BreezSdk {
    /// Check whether a username is free for this wallet to register.
    ///
    /// The check is signed with the wallet's identity key, so every call costs
    /// a signing operation and a server round trip: run it when the user
    /// finishes typing, not on every keystroke.
    pub async fn check_lightning_address_available(
        &self,
        req: CheckLightningAddressRequest,
    ) -> Result<bool, SdkError> {
        let Some(client) = &self.lnurl_server_client else {
            return Err(SdkError::Generic(
                "LNURL server is not configured".to_string(),
            ));
        };

        let username = sanitize_username(&req.username);
        let available = client.check_username_available(&username).await?;
        Ok(available)
    }

    pub async fn get_lightning_address(&self) -> Result<Option<LightningAddressInfo>, SdkError> {
        let cache = ObjectCacheRepository::new(self.storage.clone());
        let cached = cache.fetch_lightning_address().await?;
        if cached.is_none() && self.lnurl_server_client.is_some() {
            return self.recover_lightning_address().await;
        }
        Ok(cached.flatten())
    }

    pub async fn register_lightning_address(
        &self,
        request: RegisterLightningAddressRequest,
    ) -> Result<LightningAddressInfo, SdkError> {
        let cache = ObjectCacheRepository::new(self.storage.clone());
        let Some(client) = &self.lnurl_server_client else {
            return Err(SdkError::Generic(
                "LNURL server is not configured".to_string(),
            ));
        };

        let username = sanitize_username(&request.username);

        let description = match request.description {
            Some(description) => description,
            None => default_description(&username, client.domain()),
        };

        let params = crate::lnurl::RegisterLightningAddressRequest {
            username: username.clone(),
            description: description.clone(),
        };

        let response = client.register_lightning_address(&params).await?;
        let address_info = LightningAddressInfo {
            lightning_address: response.lightning_address,
            description,
            lnurl: LnurlInfo::new(response.lnurl),
            username,
        };
        cache.save_lightning_address(&address_info, false).await?;
        Ok(address_info)
    }

    /// Authorize transferring the current owner's registered lightning address
    /// username to `request.transferee_pubkey`. Returns a
    /// [`TransferAuthorization`] to hand to the new owner, who
    /// claims it via [`BreezSdk::claim_lightning_address_transfer`].
    /// Errors if the current owner has no lightning address registered.
    pub async fn authorize_lightning_address_transfer(
        &self,
        request: AuthorizeTransferRequest,
    ) -> Result<TransferAuthorization, SdkError> {
        let cache = ObjectCacheRepository::new(self.storage.clone());
        let Some(address_info) = cache.fetch_lightning_address().await?.flatten() else {
            return Err(SdkError::Generic(
                "No lightning address registered to transfer".to_string(),
            ));
        };

        // The domain the address is registered on, not this SDK's configured
        // one. The transferee submits the transfer, so what the signature has to
        // cover is the domain their server resolves, and this SDK's own
        // configuration is not part of that: an address cached from a domain it
        // no longer points at is still a registration it can hand over.
        let domain = address_domain(&address_info.lightning_address)?;

        let self_pubkey = self.spark_wallet.get_identity_public_key().to_string();
        let transferee_pubkey = normalized_pubkey(&request.transferee_pubkey)?;
        let timestamp = now_secs()?;
        let signature = self
            .spark_wallet
            .sign_message(&signed_message::transfer_from(
                &domain,
                &address_info.username,
                &self_pubkey,
                &transferee_pubkey,
                timestamp,
            ))
            .await?;

        Ok(TransferAuthorization {
            username: address_info.username,
            pubkey: self_pubkey,
            signature: signature.serialize_der().to_lower_hex_string(),
            domain,
            timestamp,
        })
    }

    /// Claim a lightning address username handed over by its current owner,
    /// using the [`TransferAuthorization`] from
    /// [`BreezSdk::authorize_lightning_address_transfer`]. Completes the
    /// takeover and returns the newly-owned address.
    pub async fn claim_lightning_address_transfer(
        &self,
        request: ClaimTransferRequest,
    ) -> Result<LightningAddressInfo, SdkError> {
        let cache = ObjectCacheRepository::new(self.storage.clone());
        let Some(client) = &self.lnurl_server_client else {
            return Err(SdkError::Generic(
                "LNURL server is not configured".to_string(),
            ));
        };

        // Checked before the round trip so the failure names both domains, and
        // says which one this SDK is configured for.
        require_configured_domain(
            client.domain(),
            &request.authorization.domain,
            "the authorization",
        )?;
        // The window the server enforces, applied here so an authorization the
        // transferee sat on fails saying so rather than as a generic rejection.
        // Deliberately short: nothing revokes an authorization, so expiry is the
        // only thing that takes one back.
        if now_secs()?.abs_diff(request.authorization.timestamp) > signed_message::VALIDITY_SECS {
            return Err(SdkError::InvalidInput(
                "authorization is expired or not yet valid; ask the current owner to \
                 authorize the transfer again"
                    .to_string(),
            ));
        }

        let username = sanitize_username(&request.authorization.username);
        let description = match request.description {
            Some(description) => description,
            None => default_description(&username, client.domain()),
        };

        let params = crate::lnurl::TransferLightningAddressRequest {
            username: username.clone(),
            description: description.clone(),
            from_pubkey: normalized_pubkey(&request.authorization.pubkey)?,
            from_signature: request.authorization.signature,
            timestamp: request.authorization.timestamp,
        };

        let response = client.transfer_lightning_address(&params).await?;
        let address_info = LightningAddressInfo {
            lightning_address: response.lightning_address,
            description,
            lnurl: LnurlInfo::new(response.lnurl),
            username,
        };
        cache.save_lightning_address(&address_info, false).await?;
        Ok(address_info)
    }

    /// Give up this wallet's lightning address.
    ///
    /// The server holds the address for this wallet afterwards: while the hold
    /// stands only this wallet can register it, so payers who kept the old
    /// address are not redirected to someone else. How long a hold stands is
    /// the server's policy.
    pub async fn delete_lightning_address(&self) -> Result<(), SdkError> {
        let cache = ObjectCacheRepository::new(self.storage.clone());
        let Some(address_info) = cache.fetch_lightning_address().await?.flatten() else {
            return Ok(());
        };

        let Some(client) = &self.lnurl_server_client else {
            return Err(SdkError::Generic(
                "LNURL server is not configured".to_string(),
            ));
        };

        let params = crate::lnurl::UnregisterLightningAddressRequest {
            username: address_info.username,
        };

        match client.unregister_lightning_address(&params).await {
            Ok(()) => {}
            // A 409 is either a name this wallet no longer holds (another
            // device re-registered under the same identity key) or a statement
            // the server already acted on. Resync settles both: it re-caches an
            // address that is still there and clears one that is gone, so a
            // retry either signs the real address or short-circuits.
            Err(
                e @ LnurlServerError::Network {
                    statuscode: 409, ..
                },
            ) => {
                self.recover_lightning_address().await?;
                return Err(e.into());
            }
            Err(e) => return Err(e.into()),
        }

        cache.delete_lightning_address(false).await?;
        Ok(())
    }
}

// Private lightning address methods
impl BreezSdk {
    /// Attempts to recover a lightning address from the lnurl server.
    pub(super) async fn recover_lightning_address(
        &self,
    ) -> Result<Option<LightningAddressInfo>, SdkError> {
        let cache = ObjectCacheRepository::new(self.storage.clone());

        let Some(client) = &self.lnurl_server_client else {
            return Err(SdkError::Generic(
                "LNURL server is not configured".to_string(),
            ));
        };
        let resp = client.recover_lightning_address().await?;

        let result = if let Some(resp) = resp {
            let address_info = resp.into();
            cache.save_lightning_address(&address_info, true).await?;
            Some(address_info)
        } else {
            cache.delete_lightning_address(true).await?;
            None
        };

        Ok(result)
    }
}

#[cfg(test)]
#[cfg(feature = "sqlite")]
mod tests {
    use std::{path::PathBuf, sync::Arc};

    use crate::{LightningAddressInfo, LnurlInfo, persist::sqlite::SqliteStorage};

    use crate::persist::ObjectCacheRepository;

    fn create_temp_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("breez-test-{}-{}", name, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn create_temp_storage(name: &str) -> (Arc<SqliteStorage>, PathBuf) {
        let dir = create_temp_dir(name);
        let storage = SqliteStorage::new(&dir).expect("Failed to create storage");
        (Arc::new(storage), dir)
    }

    fn sample_address_info() -> LightningAddressInfo {
        LightningAddressInfo {
            lightning_address: "test@example.com".to_string(),
            username: "test".to_string(),
            description: "Test address".to_string(),
            lnurl: LnurlInfo::new("https://example.com/.well-known/lnurlp/test".to_string()),
        }
    }

    #[tokio::test]
    async fn test_fetch_returns_none_when_never_recovered() {
        let (storage, _dir) = create_temp_storage("never_recovered");
        let cache = ObjectCacheRepository::new(storage as Arc<_>);

        // Key absent -> None (never recovered)
        let result = cache.fetch_lightning_address().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_fetch_returns_some_none_after_delete() {
        let (storage, _dir) = create_temp_storage("after_delete");
        let cache = ObjectCacheRepository::new(storage as Arc<_>);

        // Save an address, then delete it
        cache
            .save_lightning_address(&sample_address_info(), false)
            .await
            .unwrap();
        cache.delete_lightning_address(false).await.unwrap();

        // Key present, value null -> Some(None) (recovered, no address)
        let result = cache.fetch_lightning_address().await.unwrap();
        assert!(
            matches!(result, Some(None)),
            "Expected Some(None) after delete"
        );
    }

    #[tokio::test]
    async fn test_fetch_returns_some_some_after_save() {
        let (storage, _dir) = create_temp_storage("after_save");
        let cache = ObjectCacheRepository::new(storage as Arc<_>);

        cache
            .save_lightning_address(&sample_address_info(), false)
            .await
            .unwrap();

        // Key present, value non-null -> Some(Some(info))
        let result = cache.fetch_lightning_address().await.unwrap();
        let info = result
            .flatten()
            .expect("Expected Some(Some(info)) after save");
        assert_eq!(info.lightning_address, "test@example.com");
    }
}

#[cfg(test)]
mod domain_tests {
    use super::{address_domain, require_configured_domain};

    /// Read from the address itself, so it names the domain the address is
    /// actually registered on rather than whatever the SDK is configured with.
    #[test]
    fn the_domain_comes_from_the_cached_address() {
        assert_eq!(address_domain("alice@example.com").unwrap(), "example.com");
        // Lowercased to match what the server resolves and lowercases.
        assert_eq!(address_domain("alice@Example.COM").unwrap(), "example.com");
        // A username may itself contain '@' in principle, so the split takes
        // the last one.
        assert_eq!(address_domain("a@b@example.com").unwrap(), "example.com");
        assert!(address_domain("not-an-address").is_err());
    }

    /// Compared against the same normalization the signed message uses, so a
    /// scheme, a mount path or a trailing slash on the configured value is not
    /// a mismatch.
    #[test]
    fn a_configured_domain_matches_however_it_was_written() {
        for configured in [
            "example.com",
            "https://example.com",
            "https://example.com/",
            "https://example.com/lnurl",
            "https://user:pass@example.com",
            "https://EXAMPLE.com",
        ] {
            assert!(
                require_configured_domain(configured, "example.com", "the authorization").is_ok(),
                "{configured}"
            );
        }
    }

    /// Both domains are named, since the caller has to see which one this SDK
    /// talks to before it can tell which side is stale.
    #[test]
    fn a_domain_that_is_not_the_configured_one_names_both() {
        let error = require_configured_domain(
            "https://example.com",
            "other.example.com",
            "the authorization",
        )
        .unwrap_err()
        .to_string();
        for expected in ["the authorization", "other.example.com", "example.com"] {
            assert!(error.contains(expected), "{error}");
        }
    }
}
