//! Minimal Nostr relay client covering what the passkey label store uses:
//! NIP-01 events and relay protocol, NIP-42 authentication and NIP-65 relay
//! lists.

use std::net::SocketAddr;
use std::sync::LazyLock;

use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::{All, Keypair, Message, Secp256k1, XOnlyPublicKey, rand, schnorr};
use futures::future::join_all;
use futures::{SinkExt, StreamExt};
use platform_utils::time::{Duration, SystemTime, UNIX_EPOCH};
use platform_utils::tokio::time::timeout;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{debug, info, warn};

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
use tokio_tungstenite::tungstenite::Message as WsMessage;
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
use tokio_tungstenite_wasm::Message as WsMessage;

#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
type WebSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
type WebSocket = tokio_tungstenite_wasm::WebSocketStream;

pub const KIND_TEXT_NOTE: u16 = 1;
pub const KIND_RELAY_LIST: u16 = 10002;
const KIND_AUTHENTICATION: u16 = 22242;

/// Bound on a relay acknowledging a published event.
const WAIT_FOR_OK_TIMEOUT: Duration = Duration::from_secs(10);
/// Bound on completing NIP-42 authentication once a relay asked for it.
const WAIT_FOR_AUTH_TIMEOUT: Duration = Duration::from_secs(7);

static SECP: LazyLock<Secp256k1<All>> = LazyLock::new(Secp256k1::new);

pub fn keypair(secret_key: &bitcoin::secp256k1::SecretKey) -> Keypair {
    Keypair::from_secret_key(&SECP, secret_key)
}

/// A signed NIP-01 event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub id: sha256::Hash,
    pub pubkey: XOnlyPublicKey,
    pub created_at: u64,
    pub kind: u16,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub sig: schnorr::Signature,
}

impl Event {
    pub fn sign(keys: &Keypair, kind: u16, tags: Vec<Vec<String>>, content: &str) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        Self::sign_at(keys, now, kind, tags, content)
    }

    fn sign_at(
        keys: &Keypair,
        created_at: u64,
        kind: u16,
        tags: Vec<Vec<String>>,
        content: &str,
    ) -> Self {
        let pubkey = keys.x_only_public_key().0;
        let id = event_id(&pubkey, created_at, kind, &tags, content);
        let sig = SECP.sign_schnorr_with_rng(
            &Message::from_digest(id.to_byte_array()),
            keys,
            &mut rand::thread_rng(),
        );
        Self {
            id,
            pubkey,
            created_at,
            kind,
            tags,
            content: content.to_string(),
            sig,
        }
    }

    /// Whether the id commits to the event fields and is signed by the author.
    pub fn verify(&self) -> bool {
        let id = event_id(
            &self.pubkey,
            self.created_at,
            self.kind,
            &self.tags,
            &self.content,
        );
        id == self.id
            && SECP
                .verify_schnorr(
                    &self.sig,
                    &Message::from_digest(id.to_byte_array()),
                    &self.pubkey,
                )
                .is_ok()
    }
}

/// NIP-01 event id: sha256 of the compact JSON array
/// `[0, pubkey, created_at, kind, tags, content]`. `serde_json` applies the
/// escaping NIP-01 prescribes, so ids match other implementations.
fn event_id(
    pubkey: &XOnlyPublicKey,
    created_at: u64,
    kind: u16,
    tags: &[Vec<String>],
    content: &str,
) -> sha256::Hash {
    let serialized = json!([0, pubkey.to_string(), created_at, kind, tags, content]).to_string();
    sha256::Hash::hash(serialized.as_bytes())
}

/// A `REQ` filter for the events of one author and kind.
#[derive(Clone, Debug, Serialize)]
pub struct Filter {
    authors: [XOnlyPublicKey; 1],
    kinds: [u16; 1],
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<usize>,
}

impl Filter {
    pub fn new(author: XOnlyPublicKey, kind: u16, limit: Option<usize>) -> Self {
        Self {
            authors: [author],
            kinds: [kind],
            limit,
        }
    }

    fn matches(&self, event: &Event) -> bool {
        self.authors[0] == event.pubkey && self.kinds[0] == event.kind
    }
}

/// Renders a relay URL the way rust-nostr does (normalized by `url`, trailing
/// slash kept only when given), or `None` for anything but a `ws`/`wss` URL.
/// The rendered form is what lands in published NIP-65 lists.
pub fn normalize_relay_url(url: &str) -> Option<String> {
    if url.matches("://").count() > 1 {
        return None;
    }
    let parsed = url::Url::parse(url)
        .ok()
        .filter(|u| matches!(u.scheme(), "ws" | "wss"))?;
    let rendered = parsed.as_str();
    let rendered = if url.ends_with('/') {
        rendered
    } else {
        rendered.trim_end_matches('/')
    };
    Some(rendered.to_string())
}

/// NIP-65 relay list naming each relay for both reading and writing.
pub fn relay_list_tags(urls: &[String]) -> Vec<Vec<String>> {
    urls.iter()
        .map(|url| vec!["r".to_string(), url.clone()])
        .collect()
}

/// The relays listed in a NIP-65 event, read the way rust-nostr reads `r`
/// tags: a `ws`/`wss` URL, optionally followed by `read` or `write`.
pub fn relay_list_urls(event: &Event) -> Vec<String> {
    event
        .tags
        .iter()
        .filter_map(|tag| match tag.as_slice() {
            [name, url, rest @ ..]
                if name == "r"
                    && (url.starts_with("ws://") || url.starts_with("wss://"))
                    && rest.first().is_none_or(|m| m == "read" || m == "write") =>
            {
                normalize_relay_url(url)
            }
            _ => None,
        })
        .collect()
}

/// How relays are reached.
#[derive(Clone)]
pub struct RelayOptions {
    /// Answers the NIP-42 challenges of relays requiring authentication.
    pub auth_keys: Keypair,
    /// SOCKS5 proxy carrying the connections. Native only.
    pub proxy: Option<SocketAddr>,
}

/// An open connection to one relay. NIP-42 challenges are answered whenever
/// the relay sends them.
pub struct Relay {
    url: String,
    socket: WebSocket,
    auth_keys: Keypair,
    auth_event: Option<sha256::Hash>,
    /// Outcome of the last authentication, once the relay acknowledged it.
    authenticated: Option<bool>,
}

impl Relay {
    async fn connect(url: &str, options: &RelayOptions) -> Result<Self, String> {
        Ok(Self {
            url: url.to_string(),
            socket: open_websocket(url, options.proxy).await?,
            auth_keys: options.auth_keys,
            auth_event: None,
            authenticated: None,
        })
    }

    /// Stored events matching `filter`, up to the relay's `EOSE`. Events that
    /// fail verification or do not match the filter are dropped.
    async fn fetch(&mut self, filter: &Filter) -> Result<Vec<Event>, String> {
        let subscription_id = format!("{:016x}", rand::random::<u64>());
        let req = json!(["REQ", subscription_id, filter]);
        self.send(&req).await?;

        let mut events = Vec::new();
        let mut retried = false;
        loop {
            let message = self.recv().await?;
            if message[1] != subscription_id.as_str() {
                continue;
            }
            match message[0].as_str() {
                Some("EVENT") => match serde_json::from_value::<Event>(message[2].clone()) {
                    Ok(event) if filter.matches(&event) && event.verify() => events.push(event),
                    _ => warn!(relay = %self.url, "Dropping an invalid event"),
                },
                Some("EOSE") => {
                    // The events are in hand: a failed CLOSE does not matter.
                    let _ = self.send(&json!(["CLOSE", subscription_id])).await;
                    return Ok(events);
                }
                Some("CLOSED") => {
                    let reason = message[2].as_str().unwrap_or_default();
                    if reason.starts_with("auth-required:") && !retried {
                        self.wait_for_auth().await?;
                        retried = true;
                        self.send(&req).await?;
                    } else if reason.is_empty() {
                        return Ok(events);
                    } else {
                        return Err(format!("subscription closed: {reason}"));
                    }
                }
                _ => {}
            }
        }
    }

    /// Publishes `event`, succeeding once the relay accepts it.
    async fn publish(&mut self, event: &Event) -> Result<(), String> {
        let message = json!(["EVENT", event]);
        let id = event.id.to_string();
        let publish = async {
            self.send(&message).await?;
            let mut retried = false;
            loop {
                let reply = self.recv().await?;
                if reply[0] != "OK" || reply[1] != id.as_str() {
                    continue;
                }
                let reason = reply[3].as_str().unwrap_or_default();
                // A relay already holding the event has what we wanted.
                if reply[2] == true || reason.starts_with("duplicate:") {
                    return Ok(());
                }
                if retried || !reason.starts_with("auth-required:") {
                    return Err(format!("event rejected: {reason}"));
                }
                self.wait_for_auth().await?;
                retried = true;
                self.send(&message).await?;
            }
        };
        timeout(WAIT_FOR_OK_TIMEOUT, publish)
            .await
            .unwrap_or_else(|_| Err("timed out waiting for OK".to_string()))
    }

    async fn close(mut self) {
        let _ = timeout(Duration::from_secs(1), SinkExt::close(&mut self.socket)).await;
    }

    async fn send(&mut self, message: &Value) -> Result<(), String> {
        self.socket
            .send(WsMessage::text(message.to_string()))
            .await
            .map_err(|e| e.to_string())
    }

    /// The next relay message, answering NIP-42 challenges along the way.
    async fn recv(&mut self) -> Result<Value, String> {
        loop {
            let text = match self.socket.next().await {
                Some(Ok(WsMessage::Text(text))) => text,
                Some(Ok(WsMessage::Close(_))) | None => return Err("connection closed".into()),
                Some(Err(e)) => return Err(e.to_string()),
                // Natively, tungstenite answers pings itself.
                Some(Ok(_)) => continue,
            };
            let Ok(message) = serde_json::from_str::<Value>(text.as_str()) else {
                continue;
            };
            match message[0].as_str() {
                Some("AUTH") => {
                    let challenge = message[1].as_str().unwrap_or_default();
                    let tags = vec![
                        vec!["challenge".to_string(), challenge.to_string()],
                        vec!["relay".to_string(), self.url.clone()],
                    ];
                    let event = Event::sign(&self.auth_keys, KIND_AUTHENTICATION, tags, "");
                    self.auth_event = Some(event.id);
                    self.authenticated = None;
                    self.send(&json!(["AUTH", event])).await?;
                }
                Some("OK")
                    if self
                        .auth_event
                        .is_some_and(|id| message[1] == id.to_string()) =>
                {
                    let accepted = message[2] == true;
                    if accepted {
                        info!(relay = %self.url, "Authenticated to relay");
                    } else {
                        warn!(relay = %self.url, "Relay authentication failed: {}", message[3]);
                    }
                    self.authenticated = Some(accepted);
                    // Surfaced so a caller waiting on the outcome wakes up.
                    return Ok(message);
                }
                Some("NOTICE") => debug!(relay = %self.url, "Relay notice: {}", message[1]),
                _ => return Ok(message),
            }
        }
    }

    /// Waits for the relay to acknowledge our NIP-42 authentication. Messages
    /// arriving meanwhile are dropped: the caller's request was refused.
    async fn wait_for_auth(&mut self) -> Result<(), String> {
        let wait = async {
            loop {
                match self.authenticated {
                    Some(true) => return Ok(()),
                    Some(false) => return Err("relay authentication failed".to_string()),
                    None => {
                        self.recv().await?;
                    }
                }
            }
        };
        timeout(WAIT_FOR_AUTH_TIMEOUT, wait)
            .await
            .unwrap_or_else(|_| Err("timed out waiting for authentication".to_string()))
    }
}

/// Opens `url`, natively tunnelled through the SOCKS5 proxy at `proxy` when
/// given. The relay hostname is sent to the proxy as a name, so the proxy
/// resolves it.
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
async fn open_websocket(url: &str, proxy: Option<SocketAddr>) -> Result<WebSocket, String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("invalid relay url: {e}"))?;
    let host = match parsed.host() {
        Some(url::Host::Domain(domain)) => domain.to_string(),
        Some(url::Host::Ipv4(ip)) => ip.to_string(),
        Some(url::Host::Ipv6(ip)) => ip.to_string(),
        None => return Err("relay url has no host".to_string()),
    };
    let port = parsed
        .port_or_known_default()
        .ok_or("relay url has no port")?;
    let tcp = match proxy {
        None => tokio::net::TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|e| e.to_string())?,
        Some(proxy) => tokio_socks::tcp::Socks5Stream::connect(proxy, (host, port))
            .await
            .map_err(|e| format!("SOCKS5 via {proxy} failed: {e}"))?
            .into_inner(),
    };
    let (socket, _response) = tokio_tungstenite::client_async_tls_with_config(url, tcp, None, None)
        .await
        .map_err(|e| e.to_string())?;
    Ok(socket)
}

/// Opens `url` on the browser's `WebSocket`, which cannot be proxied.
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
async fn open_websocket(url: &str, proxy: Option<SocketAddr>) -> Result<WebSocket, String> {
    if proxy.is_some() {
        return Err("a SOCKS5 proxy cannot be honoured on WASM".to_string());
    }
    tokio_tungstenite_wasm::connect(url)
        .await
        .map_err(|e| e.to_string())
}

/// Connects to every relay of `urls` and fetches `filter` from each, each
/// relay bounded by `timeout_after`. Returns the relays that answered, still
/// open, with their events merged newest first. Fails when none answered.
pub async fn fetch(
    urls: &[String],
    options: &RelayOptions,
    filter: &Filter,
    timeout_after: Duration,
) -> Result<(Vec<Relay>, Vec<Event>), String> {
    let results = join_all(urls.iter().map(|url| async move {
        let fetched = async {
            let mut relay = Relay::connect(url, options).await?;
            let events = relay.fetch(filter).await?;
            Ok((relay, events))
        };
        timeout(timeout_after, fetched)
            .await
            .unwrap_or_else(|_| Err("timed out".to_string()))
    }))
    .await;

    let (answered, mut events) = collect(urls, results)?.into_iter().fold(
        (Vec::new(), Vec::new()),
        |(mut relays, mut events), (relay, relay_events)| {
            relays.push(relay);
            events.extend(relay_events);
            (relays, events)
        },
    );
    events.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
    events.dedup_by(|a, b| a.id == b.id);
    Ok((answered, events))
}

/// Connects to every relay of `urls`, each bounded by `timeout_after`.
/// Fails when none could be reached.
pub async fn connect(
    urls: &[String],
    options: &RelayOptions,
    timeout_after: Duration,
) -> Result<Vec<Relay>, String> {
    let results = join_all(urls.iter().map(|url| async move {
        timeout(timeout_after, Relay::connect(url, options))
            .await
            .unwrap_or_else(|_| Err("timed out".to_string()))
    }))
    .await;
    collect(urls, results)
}

/// Publishes `event` to every relay. Relays that did not accept it are
/// closed and removed. Fails when none accepted it.
pub async fn publish(relays: &mut Vec<Relay>, event: &Event) -> Result<(), String> {
    let results = join_all(relays.iter_mut().map(|relay| relay.publish(event))).await;
    let mut errors = Vec::new();
    let mut kept = Vec::new();
    for (relay, result) in relays.drain(..).zip(results) {
        match result {
            Ok(()) => kept.push(relay),
            Err(e) => {
                errors.push(format!("{}: {e}", relay.url));
                relay.close().await;
            }
        }
    }
    *relays = kept;
    if relays.is_empty() {
        return Err(errors.join("; "));
    }
    for error in errors {
        warn!("Relay failed: {error}");
    }
    Ok(())
}

pub async fn close(relays: Vec<Relay>) {
    join_all(relays.into_iter().map(Relay::close)).await;
}

/// The successes of a per-relay operation, or one error naming each relay
/// when there are none. Failures are logged when another relay succeeded,
/// since the caller then never sees them.
fn collect<T>(urls: &[String], results: Vec<Result<T, String>>) -> Result<Vec<T>, String> {
    let mut successes = Vec::new();
    let mut errors = Vec::new();
    for (url, result) in urls.iter().zip(results) {
        match result {
            Ok(value) => successes.push(value),
            Err(e) => errors.push(format!("{url}: {e}")),
        }
    }
    if successes.is_empty() {
        return Err(if errors.is_empty() {
            "no relays".to_string()
        } else {
            errors.join("; ")
        });
    }
    for error in errors {
        warn!("Relay failed: {error}");
    }
    Ok(successes)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bitcoin::secp256k1::SecretKey;

    use super::*;

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    /// Secret key 3, whose x-only public key is a well-known BIP-340 vector.
    fn fixed_keys() -> Keypair {
        let mut bytes = [0u8; 32];
        bytes[31] = 3;
        keypair(&SecretKey::from_slice(&bytes).unwrap())
    }

    // Ids below were produced by rust-nostr 0.43 for the same inputs.

    #[macros::test_all]
    fn test_text_note_id_matches_reference() {
        let keys = fixed_keys();
        assert_eq!(
            keys.x_only_public_key().0.to_string(),
            "f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9"
        );
        let event = Event::sign_at(&keys, 1_700_000_000, KIND_TEXT_NOTE, vec![], "Default");
        assert_eq!(
            event.id.to_string(),
            "7907d5d0c623c0c9c63801dedd784be5837dee661042c2545966a124755ade6a"
        );
        assert!(event.verify());
    }

    #[macros::test_all]
    fn test_id_escaping_matches_reference() {
        let content =
            "tab\there \"q\" \\ nl\n cr\r bs\u{8} ff\u{c} ctl\u{1} uni \u{e9} \u{1f680} </script>";
        let event = Event::sign_at(
            &fixed_keys(),
            1_700_000_000,
            KIND_TEXT_NOTE,
            vec![],
            content,
        );
        assert_eq!(
            event.id.to_string(),
            "ed83800b58ed6cac3a4dd9a669e85cac374e8d1724c006dd00a1e7c9327f7eb2"
        );
    }

    #[macros::test_all]
    fn test_auth_event_matches_reference() {
        // The shape `Relay::recv` signs in answer to a challenge.
        let tags = vec![
            vec!["challenge".to_string(), "challenge-123".to_string()],
            vec![
                "relay".to_string(),
                "wss://nr1.breez.technology".to_string(),
            ],
        ];
        let event = Event::sign_at(&fixed_keys(), 1_700_000_000, KIND_AUTHENTICATION, tags, "");
        assert_eq!(
            event.id.to_string(),
            "39bde66e9a8f26c3da139363b5302df493ba8828dfcd66d3ced6e397b6695fce"
        );
    }

    #[macros::test_all]
    fn test_relay_list_matches_reference() {
        let urls: Vec<String> = [
            "wss://nr1.breez.technology",
            "wss://relay.primal.net",
            "wss://relay.damus.io/",
            "wss://Relay.Example.com:443",
            "ws://example.com:8080/path",
        ]
        .iter()
        .filter_map(|url| normalize_relay_url(url))
        .collect();
        assert_eq!(
            urls,
            vec![
                "wss://nr1.breez.technology",
                "wss://relay.primal.net",
                "wss://relay.damus.io/",
                "wss://relay.example.com",
                "ws://example.com:8080/path",
            ]
        );

        let event = Event::sign_at(
            &fixed_keys(),
            1_700_000_000,
            KIND_RELAY_LIST,
            relay_list_tags(&urls),
            "",
        );
        assert_eq!(
            event.id.to_string(),
            "0f62c3237f3b0c63e82aabc86ea2b566de88f5bb253eb6f01fed4274ad6f0633"
        );
        assert_eq!(relay_list_urls(&event), urls);
    }

    #[macros::test_all]
    fn test_relay_list_urls_skips_invalid_tags() {
        let json = r#"{"id":"0000000000000000000000000000000000000000000000000000000000000000","pubkey":"f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9","created_at":1,"kind":10002,"tags":[["r","wss://a.com"],["r","wss://b.com/","read"],["r","wss://c.com","bogus"],["r","https://d.com"],["R","wss://e.com"],["r"],["p","wss://f.com"],["r","wss://g.com","write","extra"],["r","not a url"],["r","wss://h.com://x"]],"content":"","sig":"00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"}"#;
        let event: Event = serde_json::from_str(json).unwrap();
        // Same result rust-nostr 0.43 gives for this event.
        assert_eq!(
            relay_list_urls(&event),
            vec!["wss://a.com", "wss://b.com/", "wss://g.com"]
        );
    }

    #[macros::test_all]
    fn test_verify_known_events() {
        // Published events with tags and escaped content (NIP-01 vectors
        // from rust-nostr's own test suite).
        let events = [
            r#"{"content":"","created_at":1698412975,"id":"f55c30722f056e330d8a7a6a9ba1522f7522c0f1ced1c93d78ea833c78a3d6ec","kind":3,"pubkey":"f831caf722214748c72db4829986bd0cbb2bb8b3aeade1c959624a52a9629046","sig":"5092a9ffaecdae7d7794706f085ff5852befdf79df424cc3419bb797bf515ae05d4f19404cb8324b8b4380a4bd497763ac7b0f3b1b63ef4d3baa17e5f5901808","tags":[["p","4ddeb9109a8cd29ba279a637f5ec344f2479ee07df1f4043f3fe26d8948cfef9","",""],["p","bb6fd06e156929649a73e6b278af5e648214a69d88943702f1fb627c02179b95","",""],["p","b8b8210f33888fdbf5cedee9edf13c3e9638612698fe6408aff8609059053420","",""],["p","9dcee4fabcd690dc1da9abdba94afebf82e1e7614f4ea92d61d52ef9cd74e083","",""],["p","3eea9e831fefdaa8df35187a204d82edb589a36b170955ac5ca6b88340befaa0","",""],["p","885238ab4568f271b572bf48b9d6f99fa07644731f288259bd395998ee24754e","",""],["p","568a25c71fba591e39bebe309794d5c15d27dbfa7114cacb9f3586ea1314d126","",""]]}"#,
            r#"{"content":"Think about this.\n\nThe most powerful centralized institutions in the world have been replaced by a protocol that protects the individual. #bitcoin\n\nDo you doubt that we can replace everything else?\n\nBullish on the future of humanity\nnostr:nevent1qqs9ljegkuk2m2ewfjlhxy054n6ld5dfngwzuep0ddhs64gc49q0nmqpzdmhxue69uhhyetvv9ukzcnvv5hx7un8qgsw3mfhnrr0l6ll5zzsrtpeufckv2lazc8k3ru5c3wkjtv8vlwngksrqsqqqqqpttgr27","created_at":1703184271,"id":"38acf9b08d06859e49237688a9fd6558c448766f47457236c2331f93538992c6","kind":1,"pubkey":"e8ed3798c6ffebffa08501ac39e271662bfd160f688f94c45d692d8767dd345a","sig":"f76d5ecc8e7de688ac12b9d19edaacdcffb8f0c8fa2a44c00767363af3f04dbc069542ddc5d2f63c94cb5e6ce701589d538cf2db3b1f1211a96596fabb6ecafe","tags":[["e","5fcb28b72cadab2e4cbf7311f4acf5f6d1a99a1c2e642f6b6f0d5518a940f9ec","","mention"],["p","e8ed3798c6ffebffa08501ac39e271662bfd160f688f94c45d692d8767dd345a","","mention"],["t","bitcoin"],["t","bitcoin"]]}"#,
        ];
        for json in events {
            let event: Event = serde_json::from_str(json).unwrap();
            assert!(event.verify());
        }
    }

    #[macros::test_all]
    fn test_verify_rejects_tampering() {
        let keys = fixed_keys();
        let event = Event::sign(&keys, KIND_TEXT_NOTE, vec![], "label");
        assert!(event.verify());

        let mut tampered = event.clone();
        tampered.content = "other".to_string();
        assert!(!tampered.verify());

        let other = Event::sign(&keys, KIND_TEXT_NOTE, vec![], "other");
        let mut wrong_sig = event;
        wrong_sig.sig = other.sig;
        assert!(!wrong_sig.verify());
    }

    #[macros::test_all]
    fn test_filter() {
        let keys = fixed_keys();
        let author = keys.x_only_public_key().0;
        let filter = Filter::new(author, KIND_RELAY_LIST, Some(1));
        assert_eq!(
            serde_json::to_string(&filter).unwrap(),
            r#"{"authors":["f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9"],"kinds":[10002],"limit":1}"#
        );

        let note = Event::sign(&keys, KIND_TEXT_NOTE, vec![], "x");
        assert!(Filter::new(author, KIND_TEXT_NOTE, None).matches(&note));
        assert!(!filter.matches(&note));
        let stranger = XOnlyPublicKey::from_str(
            "e8ed3798c6ffebffa08501ac39e271662bfd160f688f94c45d692d8767dd345a",
        )
        .unwrap();
        assert!(!Filter::new(stranger, KIND_TEXT_NOTE, None).matches(&note));
    }

    /// Serves one connection the way the Breez relay does: a NIP-42
    /// challenge on connect, and `REQ`/`EVENT` refused until it is answered.
    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    async fn spawn_auth_relay() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let relay_url = url.clone();
        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let challenge = json!(["AUTH", "challenge-123"]).to_string();
            ws.send(WsMessage::text(challenge)).await.unwrap();
            let mut authenticated = false;
            while let Some(Ok(WsMessage::Text(text))) = ws.next().await {
                let message: Value = serde_json::from_str(text.as_str()).unwrap();
                let reply = match message[0].as_str().unwrap() {
                    "AUTH" => {
                        let event: Event = serde_json::from_value(message[1].clone()).unwrap();
                        authenticated = event.verify()
                            && event.kind == KIND_AUTHENTICATION
                            && event.tags[0] == ["challenge", "challenge-123"]
                            && event.tags[1] == ["relay", relay_url.as_str()];
                        json!(["OK", event.id, authenticated, ""])
                    }
                    "REQ" if authenticated => json!(["EOSE", message[1]]),
                    "REQ" => json!(["CLOSED", message[1], "auth-required: sign in"]),
                    "EVENT" if authenticated => json!(["OK", message[1]["id"], true, ""]),
                    "EVENT" => json!(["OK", message[1]["id"], false, "auth-required: sign in"]),
                    _ => continue,
                };
                ws.send(WsMessage::text(reply.to_string())).await.unwrap();
            }
        });
        url
    }

    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    #[tokio::test]
    async fn test_authenticates_and_retries_refused_requests() {
        let url = spawn_auth_relay().await;
        let keys = fixed_keys();
        let options = RelayOptions {
            auth_keys: keys,
            proxy: None,
        };
        let filter = Filter::new(keys.x_only_public_key().0, KIND_TEXT_NOTE, None);

        let (mut relays, events) = fetch(&[url], &options, &filter, Duration::from_secs(5))
            .await
            .unwrap();
        assert!(events.is_empty());
        let event = Event::sign(&keys, KIND_TEXT_NOTE, vec![], "label");
        publish(&mut relays, &event).await.unwrap();
        close(relays).await;
    }

    #[macros::test_all]
    fn test_normalize_rejects_non_relay_urls() {
        for input in ["https://relay.damus.io", "not a url", "wss://h.com://x", ""] {
            assert_eq!(normalize_relay_url(input), None, "{input}");
        }
    }
}
