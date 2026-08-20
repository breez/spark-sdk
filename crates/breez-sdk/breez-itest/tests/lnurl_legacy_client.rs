//! The pre-v2 LNURL wire format, replayed against the current server.
//!
//! Deployed wallets sign the messages this file builds, and they keep signing
//! them for as long as their release is in the field. The server's unit tests
//! already pin those bytes, but they stop at `verify_candidates`: they say
//! nothing about the JSON a wallet actually sends, the routes it addresses, or
//! what the database does with the result. That is what this file covers, over
//! HTTP against a real server and a real database.
//!
//! [`LegacyClient`] is a frozen copy of the shipped client's request shapes. It
//! is a record of what is already deployed, so it is never updated to track the
//! current client: the day it needs a change to keep passing is the day a
//! released wallet breaks.
//!
//! Delete this file at the legacy cutoff, together with the server's legacy
//! candidates.

use std::collections::HashMap;

use anyhow::{Result, anyhow};
use bitcoin::hashes::{Hash, sha256};
use bitcoin::hex::DisplayHex;
use bitcoin::secp256k1::{All, Message, PublicKey, Secp256k1, SecretKey};
use breez_sdk_itest::*;
use platform_utils::{DefaultHttpClient, HttpClient};
use rand::RngCore;
use rstest::*;
use serde_json::{Value, json};
use tracing::info;

/// The identity-key signature every pre-v2 request carries: DER, hex, over the
/// SHA256 of the raw message bytes.
fn sign(secp: &Secp256k1<All>, secret: &SecretKey, message: &str) -> String {
    let digest = sha256::Hash::hash(message.as_bytes());
    secp.sign_ecdsa(&Message::from_digest(digest.to_byte_array()), secret)
        .serialize_der()
        .to_lower_hex_string()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_secs()
}

/// Wait until the clock reads a second it has not read yet.
///
/// The pre-v2 messages carry the timestamp at second granularity and nothing
/// else that varies, so two requests of the same shape inside one second are one
/// statement, and the second is refused as a replay. Wallets re-registering a
/// name they just gave up hit this; a test doing it back to back hits it every
/// time.
async fn next_second() {
    let second = now_secs();
    while now_secs() == second {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// One HTTP exchange, kept as the status and the raw body so a test can assert
/// on a rejection as readily as on a success.
struct Exchange {
    status: u16,
    body: String,
}

impl Exchange {
    fn ok(&self, what: &str) -> Result<Value> {
        if !(200..300).contains(&self.status) {
            return Err(anyhow!(
                "{what} must succeed for a pre-v2 client, got {} {}",
                self.status,
                self.body
            ));
        }
        if self.body.is_empty() {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_str(&self.body)?)
    }
}

/// A wallet speaking the pre-v2 LNURL protocol.
struct LegacyClient {
    base_url: String,
    secp: Secp256k1<All>,
    secret: SecretKey,
    pubkey: PublicKey,
    http: DefaultHttpClient,
}

impl LegacyClient {
    fn new(base_url: &str) -> Self {
        let secp = Secp256k1::new();
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let secret = SecretKey::from_slice(&bytes).expect("32 random bytes are a valid key");
        let pubkey = secret.public_key(&secp);
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            secp,
            secret,
            pubkey,
            http: DefaultHttpClient::default(),
        }
    }

    fn pubkey_hex(&self) -> String {
        self.pubkey.to_string()
    }

    fn json_headers() -> HashMap<String, String> {
        HashMap::from([("Content-Type".to_string(), "application/json".to_string())])
    }

    /// The pre-v2 signature: the message with the timestamp appended after a
    /// hyphen, and the timestamp repeated in the request body.
    fn sign_timestamped(&self, message: &str) -> (String, u64) {
        let timestamp = now_secs();
        (
            sign(&self.secp, &self.secret, &format!("{message}-{timestamp}")),
            timestamp,
        )
    }

    async fn get(&self, path: &str) -> Result<Exchange> {
        let response = self
            .http
            .get(format!("{}{path}", self.base_url), None)
            .await
            .map_err(|e| anyhow!("{path}: {e:?}"))?;
        Ok(Exchange {
            status: response.status,
            body: response.body,
        })
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Exchange> {
        let response = self
            .http
            .post(
                format!("{}{path}", self.base_url),
                Some(Self::json_headers()),
                Some(body.to_string()),
            )
            .await
            .map_err(|e| anyhow!("{path}: {e:?}"))?;
        Ok(Exchange {
            status: response.status,
            body: response.body,
        })
    }

    async fn delete(&self, path: &str, body: &Value) -> Result<Exchange> {
        let response = self
            .http
            .delete(
                format!("{}{path}", self.base_url),
                Some(Self::json_headers()),
                Some(body.to_string()),
            )
            .await
            .map_err(|e| anyhow!("{path}: {e:?}"))?;
        Ok(Exchange {
            status: response.status,
            body: response.body,
        })
    }

    /// `GET /lnurlpay/available/{username}`, unsigned.
    async fn check_username_available(&self, username: &str) -> Result<bool> {
        let exchange = self.get(&format!("/lnurlpay/available/{username}")).await?;
        let json = exchange.ok("the availability check")?;
        json["available"]
            .as_bool()
            .ok_or_else(|| anyhow!("no 'available' field in {json}"))
    }

    /// `POST /lnurlpay/{pubkey}`, signing `"{username}-{timestamp}"`.
    async fn register(&self, username: &str, description: &str) -> Result<Exchange> {
        let (signature, timestamp) = self.sign_timestamped(username);
        self.post(
            &format!("/lnurlpay/{}", self.pubkey_hex()),
            &json!({
                "username": username,
                "signature": signature,
                "timestamp": timestamp,
                "description": description,
            }),
        )
        .await
    }

    /// `POST /lnurlpay/{pubkey}/recover`, signing `"{pubkey}-{timestamp}"`.
    async fn recover(&self) -> Result<Exchange> {
        let pubkey = self.pubkey_hex();
        let (signature, timestamp) = self.sign_timestamped(&pubkey);
        self.post(
            &format!("/lnurlpay/{pubkey}/recover"),
            &json!({ "signature": signature, "timestamp": timestamp }),
        )
        .await
    }

    /// `GET /lnurlpay/{pubkey}/metadata`, with the credential in the query
    /// string, signing the same `"{pubkey}-{timestamp}"` as recover.
    async fn list_metadata(&self) -> Result<Exchange> {
        let pubkey = self.pubkey_hex();
        let (signature, timestamp) = self.sign_timestamped(&pubkey);
        self.get(&format!(
            "/lnurlpay/{pubkey}/metadata?signature={signature}&timestamp={timestamp}"
        ))
        .await
    }

    /// `DELETE /lnurlpay/{pubkey}`, signing
    /// `"unregister:{username}-{timestamp}"`.
    async fn unregister(&self, username: &str) -> Result<Exchange> {
        let (signature, timestamp) = self.sign_timestamped(&format!("unregister:{username}"));
        self.delete(
            &format!("/lnurlpay/{}", self.pubkey_hex()),
            &json!({
                "username": username,
                "signature": signature,
                "timestamp": timestamp,
            }),
        )
        .await
    }

    /// The current owner's half of a transfer: `"transfer:{username}-{to}"`,
    /// naming neither the domain nor a time.
    fn authorize_transfer(&self, username: &str, to_pubkey: &str) -> String {
        sign(
            &self.secp,
            &self.secret,
            &format!("transfer:{username}-{to_pubkey}"),
        )
    }

    /// `POST /lnurlpay/{to_pubkey}/transfer`, with no `timestamp` field at all:
    /// the transferee signs the same bytes the current owner did.
    async fn claim_transfer(
        &self,
        username: &str,
        description: &str,
        from_pubkey: &str,
        from_signature: &str,
    ) -> Result<Exchange> {
        let to_pubkey = self.pubkey_hex();
        let to_signature = self.authorize_transfer(username, &to_pubkey);
        self.post(
            &format!("/lnurlpay/{to_pubkey}/transfer"),
            &json!({
                "username": username,
                "description": description,
                "from_pubkey": from_pubkey,
                "from_signature": from_signature,
                "to_signature": to_signature,
            }),
        )
        .await
    }
}

async fn setup_lnurl() -> LnurlFixture {
    LnurlFixture::new()
        .await
        .expect("Failed to start Lnurl service")
}

/// The whole single-wallet lifecycle over the pre-v2 wire: claim a name, read it
/// back, read the payment metadata, give the name up.
#[rstest]
#[test_log::test(tokio::test)]
async fn a_pre_v2_client_completes_the_address_lifecycle() -> Result<()> {
    let lnurl = setup_lnurl().await;
    let client = LegacyClient::new(lnurl.http_url());
    let username = "legacylifecycle";

    assert!(
        client.check_username_available(username).await?,
        "an unregistered name must read as available"
    );

    let registered = client
        .register(username, "Pay to the pre-v2 client")
        .await?
        .ok("register")?;
    assert_eq!(
        registered["lightning_address"].as_str().map(str::to_string),
        Some(format!(
            "{username}@{}",
            lnurl.http_url().trim_start_matches("http://")
        ))
    );

    assert!(
        !client.check_username_available(username).await?,
        "a registered name must read as taken"
    );

    let recovered = client.recover().await?.ok("recover")?;
    assert_eq!(recovered["username"].as_str(), Some(username));
    assert_eq!(
        recovered["description"].as_str(),
        Some("Pay to the pre-v2 client")
    );

    // The credential still travels in the query string; the current client
    // moved it into a header, and the query string is what stays supported for
    // the wallets that predate that.
    let metadata = client.list_metadata().await?.ok("list_metadata")?;
    assert!(
        metadata["metadata"].is_array(),
        "metadata must come back as a list, got {metadata}"
    );

    let unregistered = client.unregister(username).await?;
    assert!(
        (200..300).contains(&unregistered.status),
        "unregister must succeed for a pre-v2 client, got {} {}",
        unregistered.status,
        unregistered.body
    );

    let after = client.recover().await?;
    assert_eq!(
        after.status, 404,
        "the address must be gone after unregistering, got {} {}",
        after.status, after.body
    );

    info!("=== a_pre_v2_client_completes_the_address_lifecycle PASSED ===");
    Ok(())
}

/// Two pre-v2 wallets hand a username over with the untimestamped message pair,
/// which is the only authorization a deployed wallet can produce.
#[rstest]
#[test_log::test(tokio::test)]
async fn a_pre_v2_transfer_pair_still_hands_over_a_username() -> Result<()> {
    let lnurl = setup_lnurl().await;
    let alice = LegacyClient::new(lnurl.http_url());
    let bob = LegacyClient::new(lnurl.http_url());
    let username = "legacytransfer";

    alice
        .register(username, "Alice's address")
        .await?
        .ok("register")?;

    let authorization = alice.authorize_transfer(username, &bob.pubkey_hex());
    let transferred = bob
        .claim_transfer(
            username,
            "Bob's address",
            &alice.pubkey_hex(),
            &authorization,
        )
        .await?
        .ok("transfer")?;
    assert!(
        transferred["lightning_address"]
            .as_str()
            .is_some_and(|address| address.starts_with(username)),
        "the transfer must return the handed-over address, got {transferred}"
    );

    let bob_holds = bob.recover().await?.ok("recover after transfer")?;
    assert_eq!(bob_holds["username"].as_str(), Some(username));

    let alice_holds = alice.recover().await?;
    assert_eq!(
        alice_holds.status, 404,
        "the previous owner must hold nothing after the handover"
    );

    // Nothing bounds the pre-v2 pair in time, so the claim on it is what stops
    // it running a second time.
    let replayed = bob
        .claim_transfer(
            username,
            "Bob's address",
            &alice.pubkey_hex(),
            &authorization,
        )
        .await?;
    assert_eq!(
        replayed.status, 409,
        "the same authorization must not act twice, got {} {}",
        replayed.status, replayed.body
    );

    info!("=== a_pre_v2_transfer_pair_still_hands_over_a_username PASSED ===");
    Ok(())
}

/// The hold a release now leaves behind is the one place a pre-v2 client sees
/// different answers from the two halves of the same question: the unsigned
/// availability route it uses cannot say a name is held *for it*, so it reads
/// the name as taken while the registration it would refuse to attempt still
/// succeeds.
///
/// Pinned because the gap is what an app gating its register button on the
/// availability check runs into: closing it means changing one of these two
/// assertions, and this test is where that decision has to be made explicitly.
#[rstest]
#[test_log::test(tokio::test)]
async fn a_pre_v2_client_can_reclaim_a_name_the_unsigned_check_calls_taken() -> Result<()> {
    let lnurl = setup_lnurl().await;
    let owner = LegacyClient::new(lnurl.http_url());
    let stranger = LegacyClient::new(lnurl.http_url());
    let username = "legacyreclaim";

    owner.register(username, "First").await?.ok("register")?;
    owner.unregister(username).await?.ok("unregister")?;

    assert!(
        !owner.check_username_available(username).await?,
        "the unsigned check answers for nobody, so a released name reads as taken \
         even to the wallet it is held for"
    );
    assert!(
        !stranger.check_username_available(username).await?,
        "and as taken to everyone else"
    );

    let sniped = stranger.register(username, "Sniped").await?;
    assert_eq!(
        sniped.status, 409,
        "a stranger must not take a held name, got {} {}",
        sniped.status, sniped.body
    );

    // Otherwise the reclaim re-signs the bytes the first register already
    // spent, and the replay check answers before the hold does.
    next_second().await;
    let reclaimed = owner.register(username, "Reclaimed").await?;
    assert!(
        (200..300).contains(&reclaimed.status),
        "the wallet the name is held for must still be able to register it, got {} {}",
        reclaimed.status,
        reclaimed.body
    );

    info!("=== a_pre_v2_client_can_reclaim_a_name_the_unsigned_check_calls_taken PASSED ===");
    Ok(())
}

/// The v2 transfer message the current SDK's `authorize_lightning_address_transfer`
/// produces, written out rather than built from the shared helper: this file
/// stands for what is on the wire, not for what the current code composes.
fn v2_transfer_from(
    domain: &str,
    username: &str,
    from_pubkey: &str,
    to_pubkey: &str,
    timestamp: u64,
) -> String {
    format!(
        "breez-lnurl:v2\ntransfer-from\n{domain}\n{username}\n{from_pubkey}\n{to_pubkey}\n{timestamp}"
    )
}

/// A transfer needs both wallets on the same side of the v2 change: the
/// `timestamp` field is what picks which messages the server rebuilds, and it
/// picks one set for both signatures at once.
///
/// Neither direction is an accident, and neither is silently degraded: the
/// server refuses rather than falling back, which is what stops a v2
/// authorization being downgraded by stripping its timestamp. The cost is that
/// a handover between a pre-v2 wallet and a current one fails, and this is where
/// that cost is recorded.
#[rstest]
#[test_log::test(tokio::test)]
async fn a_transfer_across_the_v2_boundary_is_refused_in_both_directions() -> Result<()> {
    let lnurl = setup_lnurl().await;
    let domain = lnurl.http_url().trim_start_matches("http://").to_string();
    let alice = LegacyClient::new(lnurl.http_url());
    let bob = LegacyClient::new(lnurl.http_url());

    // A pre-v2 wallet authorizes; a current wallet claims, and so sends the
    // timestamp that selects the v2 messages.
    let username = "boundaryoldnew";
    alice
        .register(username, "Alice's address")
        .await?
        .ok("register")?;
    let legacy_authorization = alice.authorize_transfer(username, &bob.pubkey_hex());
    let to_pubkey = bob.pubkey_hex();
    let timestamp = now_secs();
    let refused = bob
        .post(
            &format!("/lnurlpay/{to_pubkey}/transfer"),
            &json!({
                "username": username,
                "description": "Bob's address",
                "from_pubkey": alice.pubkey_hex(),
                "from_signature": legacy_authorization,
                "to_signature": bob.authorize_transfer(username, &to_pubkey),
                "timestamp": timestamp,
            }),
        )
        .await?;
    assert_eq!(
        refused.status, 400,
        "a pre-v2 authorization must not verify against the v2 messages, got {} {}",
        refused.status, refused.body
    );

    // And the other way: a current wallet authorizes, a pre-v2 wallet claims and
    // sends no timestamp at all.
    let username = "boundarynewold";
    let carol = LegacyClient::new(lnurl.http_url());
    let dave = LegacyClient::new(lnurl.http_url());
    carol
        .register(username, "Carol's address")
        .await?
        .ok("register")?;
    let timestamp = now_secs();
    let v2_authorization = sign(
        &carol.secp,
        &carol.secret,
        &v2_transfer_from(
            &domain,
            username,
            &carol.pubkey_hex(),
            &dave.pubkey_hex(),
            timestamp,
        ),
    );
    let refused = dave
        .claim_transfer(
            username,
            "Dave's address",
            &carol.pubkey_hex(),
            &v2_authorization,
        )
        .await?;
    assert_eq!(
        refused.status, 400,
        "a v2 authorization must not verify against the pre-v2 message, got {} {}",
        refused.status, refused.body
    );

    info!("=== a_transfer_across_the_v2_boundary_is_refused_in_both_directions PASSED ===");
    Ok(())
}
