//! Domains the server handles as another domain.
//!
//! A row in `domain_aliases` makes a domain an alias of another, its target.
//! Unless the server refuses the alias, [`rewrite_domain`] turns a request for
//! the alias into a request for the target, and makes the response name the
//! alias again. The handlers behind it work with the target, and read
//! [`ActualDomain`] where the domain the client knows matters.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
    time::Duration,
};

use axum::{
    body::{Body, to_bytes},
    extract::{FromRequestParts, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::Value;
use tokio::sync::RwLock;
use tracing::{error, info};

use crate::repository::{DomainAliasConfig, LnurlRepository, LnurlRepositoryError};
use crate::routes::Host;

const REFRESH_INTERVAL: Duration = Duration::from_mins(1);

/// Every alias, by name.
pub type DomainAliases = HashMap<String, Alias>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alias {
    pub target: String,
    /// Why the server does not serve the domain. `None` when it does.
    pub refusal: Option<Refusal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Payers of an address under the domain would pay the target's user of
    /// the same name.
    HasNames,
    TargetIsAlias,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::HasNames => "usernames are registered or held under the domain itself",
            Self::TargetIsAlias => "its target is an alias too",
        })
    }
}

/// The domain a request was sent to. It is the domain the client knows: a
/// wallet signs it and a payer reads it in an address. `Host` names the target
/// instead when the domain is an alias.
#[derive(Debug, Clone)]
pub struct ActualDomain(pub String);

/// A response with this extension holds text the server did not write, so
/// [`rewrite_domain`] leaves the domains in it as they are.
#[derive(Debug, Clone, Copy)]
pub struct Verbatim;

/// Load the aliases from the database and start a background task that
/// refreshes them. Returns the handle [`rewrite_domain`] reads.
pub async fn start<DB>(db: DB) -> Result<Arc<RwLock<DomainAliases>>, LnurlRepositoryError>
where
    DB: LnurlRepository + Send + Sync + 'static,
{
    let initial = load(&db).await?;
    log_changes(&DomainAliases::new(), &initial);
    let aliases = Arc::new(RwLock::new(initial));

    let refreshed = Arc::clone(&aliases);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(REFRESH_INTERVAL).await;
            refresh_once(&db, &refreshed).await;
        }
    });

    Ok(aliases)
}

async fn load<DB>(db: &DB) -> Result<DomainAliases, LnurlRepositoryError>
where
    DB: LnurlRepository,
{
    Ok(to_map(db.list_domain_aliases().await?))
}

fn to_map(configs: Vec<DomainAliasConfig>) -> DomainAliases {
    let aliases: HashSet<String> = configs.iter().map(|config| config.domain.clone()).collect();
    configs
        .into_iter()
        .map(|config| {
            let refusal = refusal(&config, &aliases);
            let alias = Alias {
                target: config.target_domain,
                refusal,
            };
            (config.domain, alias)
        })
        .collect()
}

fn refusal(config: &DomainAliasConfig, aliases: &HashSet<String>) -> Option<Refusal> {
    if aliases.contains(&config.target_domain) {
        return Some(Refusal::TargetIsAlias);
    }
    if config.has_names {
        return Some(Refusal::HasNames);
    }
    None
}

async fn refresh_once<DB>(db: &DB, aliases: &RwLock<DomainAliases>)
where
    DB: LnurlRepository,
{
    let latest = match load(db).await {
        Ok(latest) => latest,
        Err(e) => {
            error!("Failed to refresh domain aliases: {e}");
            return;
        }
    };

    {
        let current = aliases.read().await;
        if *current == latest {
            return;
        }
        log_changes(&current, &latest);
    }

    *aliases.write().await = latest;
}

fn log_changes(current: &DomainAliases, latest: &DomainAliases) {
    for (domain, alias) in latest {
        if current.get(domain) == Some(alias) {
            continue;
        }
        let target = &alias.target;
        match alias.refusal {
            None => info!("domain '{domain}' is an alias of '{target}'"),
            Some(refusal) => {
                error!("not serving domain '{domain}', an alias of '{target}': {refusal}");
            }
        }
    }
    for domain in current.keys() {
        if !latest.contains_key(domain) {
            info!("domain alias removed: {domain}");
        }
    }
}

/// Handles a request for an alias as a request for its target, and names the
/// alias again in the response. A request for an alias the server refuses
/// stays as it is.
pub async fn rewrite_domain(
    State(aliases): State<Arc<RwLock<DomainAliases>>>,
    request: Request,
    next: Next,
) -> Response {
    let (mut parts, body) = request.into_parts();
    let Ok(Host(host)) = Host::from_request_parts(&mut parts, &()).await else {
        return next.run(Request::from_parts(parts, body)).await;
    };
    let actual = host.trim().to_lowercase();
    parts.extensions.insert(ActualDomain(actual.clone()));

    let alias = aliases.read().await.get(&actual).cloned();
    let Some(Alias {
        target,
        refusal: None,
    }) = alias
    else {
        return next.run(Request::from_parts(parts, body)).await;
    };
    let Ok(host) = HeaderValue::from_str(&target) else {
        error!("domain '{actual}' is an alias of '{target}', which is no host");
        return next.run(Request::from_parts(parts, body)).await;
    };
    parts.headers.insert(header::HOST, host);

    let response = next.run(Request::from_parts(parts, body)).await;
    name_actual_domain(response, &target, &actual).await
}

/// Applies [`replace_domain`] to every string value in `response`'s JSON body,
/// so that it names `actual` instead of `target`.
async fn name_actual_domain(response: Response, target: &str, actual: &str) -> Response {
    if response.extensions().get::<Verbatim>().is_some() {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let bytes = match to_bytes(body, usize::MAX).await {
        Ok(bytes) => bytes,
        Err(e) => {
            error!("failed to read a response body: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let Ok(mut json) = serde_json::from_slice::<Value>(&bytes) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    replace_domain_in_strings(&mut json, target, actual);
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(json.to_string()))
}

fn replace_domain_in_strings(json: &mut Value, from: &str, to: &str) {
    match json {
        Value::String(text) => *text = replace_domain(text, from, to),
        Value::Array(items) => {
            for item in items {
                replace_domain_in_strings(item, from, to);
            }
        }
        Value::Object(fields) => {
            for field in fields.values_mut() {
                replace_domain_in_strings(field, from, to);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// `text` with `to` in place of `from` wherever `from` follows `@` or `://`
/// and no longer host name or port continues it. Those are the places of the
/// domain in an address and of the host in a URL.
///
/// A username can be spelled like a domain, but holds no `@` and no `://`, so
/// a username is never replaced.
pub fn replace_domain(text: &str, from: &str, to: &str) -> String {
    if from.is_empty() || from == to {
        return text.to_string();
    }
    // Lowercasing ASCII moves no byte, so offsets found here fit `text`.
    let lowercase = text.to_ascii_lowercase();
    let mut replaced = String::with_capacity(text.len());
    let mut copied = 0;
    for (start, _) in lowercase.match_indices(from) {
        let end = start.saturating_add(from.len());
        if follows_at_or_scheme(&lowercase[..start]) && !continues_host(&lowercase[end..]) {
            replaced.push_str(&text[copied..start]);
            replaced.push_str(to);
            copied = end;
        }
    }
    replaced.push_str(&text[copied..]);
    replaced
}

fn follows_at_or_scheme(before: &str) -> bool {
    before.ends_with('@') || before.ends_with("://")
}

/// Whether `after`, the text behind a domain's name, makes that name the start
/// of a longer host: more of the label, another label, or a port.
fn continues_host(after: &str) -> bool {
    let mut chars = after.chars();
    match (chars.next(), chars.next()) {
        (Some(first), _) if first.is_ascii_alphanumeric() || first == '-' => true,
        (Some('.'), Some(second)) => second.is_ascii_alphanumeric(),
        (Some(':'), Some(second)) => second.is_ascii_digit(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use axum::{Extension, Json, Router, http::Method, routing::get};
    use bitcoin::secp256k1::{PublicKey, SecretKey};
    use lnurl_models::{
        CheckUsernameAvailableRequest, RecoverLnurlPayRequest, RecoverLnurlPayResponse,
        RegisterLnurlPayRequest, RegisterLnurlPayResponse, TransferLnurlPayRequest,
        TransferLnurlPayResponse, UnregisterLnurlPayRequest, signed_message,
    };
    use serde_json::json;
    use spark_postgres::deadpool_postgres::Pool;
    use testcontainers::ContainerAsync;
    use testcontainers_modules::postgres::Postgres;

    use super::*;
    use crate::postgresql::LnurlRepository as PostgresRepository;
    use crate::test_support::{TestServer, sign, test_pool, test_state, transfer_key};
    use crate::time::now_u64;

    const TARGET: &str = "tips.example";
    const ALIAS: &str = "cash.example";

    fn to_alias(text: &str) -> String {
        replace_domain(text, TARGET, ALIAS)
    }

    #[test]
    fn replaces_the_domain_of_an_address_and_the_host_of_a_url() {
        for (text, expected) in [
            ("alice@tips.example", "alice@cash.example"),
            (
                "lnurlp://tips.example/lnurlp/alice",
                "lnurlp://cash.example/lnurlp/alice",
            ),
            (
                "https://tips.example/lnurlp/verify/00",
                "https://cash.example/lnurlp/verify/00",
            ),
            ("Pay to alice@tips.example.", "Pay to alice@cash.example."),
            ("Alice@Tips.Example", "Alice@cash.example"),
            (
                "alice@tips.example or https://tips.example/alice",
                "alice@cash.example or https://cash.example/alice",
            ),
            (
                r#"[["text/plain","Tips\nfor alice@tips.example"],["text/identifier","alice@tips.example"]]"#,
                r#"[["text/plain","Tips\nfor alice@cash.example"],["text/identifier","alice@cash.example"]]"#,
            ),
        ] {
            assert_eq!(to_alias(text), expected);
        }
    }

    #[test]
    fn leaves_a_username_spelled_like_the_domain() {
        assert_eq!(to_alias("tips.example"), "tips.example");
        assert_eq!(
            to_alias("tips.example@tips.example"),
            "tips.example@cash.example"
        );
        assert_eq!(
            to_alias("lnurlp://tips.example/lnurlp/tips.example"),
            "lnurlp://cash.example/lnurlp/tips.example"
        );
    }

    #[test]
    fn leaves_other_hosts() {
        for text in [
            "alice@tips.example.org",
            "alice@tips.examples",
            "alice@tips.example-two",
            "alice@my.tips.example",
            "https://mytips.example/",
            "https://tips.example:8443/",
            "We like tips.example here",
        ] {
            assert_eq!(to_alias(text), text);
        }
    }

    #[test]
    fn replaces_each_occurrence_by_a_name_of_another_length() {
        let text = "a@tips.example, b@tips.example and https://tips.example/c";
        assert_eq!(
            replace_domain(text, TARGET, "x.io"),
            "a@x.io, b@x.io and https://x.io/c"
        );
        assert_eq!(
            replace_domain(text, TARGET, "a-longer-domain.example"),
            "a@a-longer-domain.example, b@a-longer-domain.example and \
             https://a-longer-domain.example/c"
        );
    }

    #[test]
    fn a_domain_with_a_port_is_replaced_whole() {
        let replaced = |text| replace_domain(text, "localhost:8080", "localhost:9090");
        assert_eq!(
            replaced("http://localhost:8080/lnurlp/alice"),
            "http://localhost:9090/lnurlp/alice"
        );
        assert_eq!(
            replaced("http://localhost:80801/"),
            "http://localhost:80801/"
        );
    }

    fn config(domain: &str, target_domain: &str) -> DomainAliasConfig {
        DomainAliasConfig {
            domain: domain.to_string(),
            target_domain: target_domain.to_string(),
            has_names: false,
        }
    }

    #[test]
    fn an_alias_names_its_target() {
        let aliases = to_map(vec![config("a.example", "b.example")]);
        assert_eq!(
            aliases["a.example"],
            Alias {
                target: "b.example".to_string(),
                refusal: None,
            }
        );
        assert!(!aliases.contains_key("b.example"));
    }

    #[test]
    fn a_domain_with_usernames_of_its_own_is_not_served() {
        let aliases = to_map(vec![DomainAliasConfig {
            has_names: true,
            ..config("a.example", "b.example")
        }]);
        assert_eq!(aliases["a.example"].refusal, Some(Refusal::HasNames));
    }

    #[test]
    fn an_alias_of_an_alias_is_not_served() {
        let aliases = to_map(vec![
            config("a.example", "b.example"),
            config("b.example", "c.example"),
        ]);
        assert_eq!(aliases["a.example"].refusal, Some(Refusal::TargetIsAlias));
        assert_eq!(aliases["b.example"].refusal, None);
    }

    #[test]
    fn a_loop_of_aliases_is_not_served() {
        let aliases = to_map(vec![
            config("a.example", "b.example"),
            config("b.example", "a.example"),
            config("c.example", "c.example"),
        ]);
        for domain in ["a.example", "b.example", "c.example"] {
            assert_eq!(aliases[domain].refusal, Some(Refusal::TargetIsAlias));
        }
    }

    /// A server behind `aliases` whose routes report the `Host` and the
    /// actual domain their handler was given.
    async fn echo_server(aliases: DomainAliases) -> TestServer {
        async fn echo(
            Host(host): Host,
            Extension(ActualDomain(actual)): Extension<ActualDomain>,
        ) -> Json<Value> {
            Json(json!({
                "host": host,
                "actual": actual,
                "addresses": [format!("alice@{host}")],
            }))
        }
        async fn verbatim(Host(host): Host) -> impl IntoResponse {
            (
                Extension(Verbatim),
                Json(json!({ "address": format!("alice@{host}") })),
            )
        }
        let router = Router::new()
            .route("/echo", get(echo))
            .route("/verbatim", get(verbatim))
            .layer(axum::middleware::from_fn_with_state(
                Arc::new(RwLock::new(aliases)),
                rewrite_domain,
            ));
        TestServer::start(router).await
    }

    fn alias(target: &str, refusal: Option<Refusal>) -> Alias {
        Alias {
            target: target.to_string(),
            refusal,
        }
    }

    async fn get_json(server: &TestServer, domain: &str, path: &str) -> Value {
        server
            .request(Method::GET, domain, path)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_request_for_an_alias_is_handled_as_its_target() {
        let server = echo_server(DomainAliases::from([(
            ALIAS.to_string(),
            alias(TARGET, None),
        )]))
        .await;

        let echoed = get_json(&server, "Cash.Example", "/echo").await;
        assert_eq!(
            echoed,
            json!({
                "host": TARGET,
                "actual": ALIAS,
                "addresses": ["alice@cash.example"],
            })
        );
    }

    #[tokio::test]
    async fn a_response_arrives_whole_when_the_domains_differ_in_length() {
        let aliases = ["a-much-longer-domain.example", "x.io"];
        let server = echo_server(
            aliases
                .iter()
                .map(|domain| (domain.to_string(), alias(TARGET, None)))
                .collect(),
        )
        .await;

        for domain in aliases {
            let echoed = get_json(&server, domain, "/echo").await;
            assert_eq!(echoed["addresses"], json!([format!("alice@{domain}")]));
        }
    }

    #[tokio::test]
    async fn a_request_for_any_other_domain_is_left_as_it_is() {
        let server = echo_server(DomainAliases::from([(
            ALIAS.to_string(),
            alias(TARGET, None),
        )]))
        .await;

        let echoed = get_json(&server, "Tips.Example", "/echo").await;
        assert_eq!(
            echoed,
            json!({
                "host": "Tips.Example",
                "actual": TARGET,
                "addresses": ["alice@Tips.Example"],
            })
        );
    }

    #[tokio::test]
    async fn a_request_for_a_refused_alias_is_left_as_it_is() {
        let server = echo_server(DomainAliases::from([(
            ALIAS.to_string(),
            alias(TARGET, Some(Refusal::HasNames)),
        )]))
        .await;

        let echoed = get_json(&server, ALIAS, "/echo").await;
        assert_eq!(
            echoed,
            json!({
                "host": ALIAS,
                "actual": ALIAS,
                "addresses": ["alice@cash.example"],
            })
        );
    }

    #[tokio::test]
    async fn a_verbatim_response_keeps_the_domains_in_it() {
        let server = echo_server(DomainAliases::from([(
            ALIAS.to_string(),
            alias(TARGET, None),
        )]))
        .await;

        let response = get_json(&server, ALIAS, "/verbatim").await;
        assert_eq!(response, json!({ "address": "alice@tips.example" }));
    }

    struct Wallet {
        secret: SecretKey,
        pubkey: PublicKey,
    }

    impl Wallet {
        fn new(seed: u8) -> Self {
            let (secret, pubkey) = transfer_key(seed);
            Self { secret, pubkey }
        }

        fn signature(&self, message: &str) -> String {
            hex::encode(sign(&self.secret, message).serialize_der())
        }

        /// Registers `username` in a request sent to `domain` and signed for
        /// `signed_domain`.
        async fn register(
            &self,
            server: &TestServer,
            domain: &str,
            signed_domain: &str,
            username: &str,
            description: &str,
        ) -> reqwest::Response {
            let timestamp = now_u64();
            let message = signed_message::register(signed_domain, username, description, timestamp);
            server
                .request(Method::POST, domain, &format!("/lnurlpay/{}", self.pubkey))
                .json(&RegisterLnurlPayRequest {
                    username: username.to_string(),
                    signature: self.signature(&message),
                    timestamp,
                    description: description.to_string(),
                })
                .send()
                .await
                .unwrap()
        }

        async fn recover(&self, server: &TestServer, domain: &str) -> RecoverLnurlPayResponse {
            let timestamp = now_u64();
            let message = signed_message::recover(domain, &self.pubkey.to_string(), timestamp);
            server
                .request(
                    Method::POST,
                    domain,
                    &format!("/lnurlpay/{}/recover", self.pubkey),
                )
                .json(&RecoverLnurlPayRequest {
                    signature: self.signature(&message),
                    timestamp,
                })
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json()
                .await
                .unwrap()
        }

        /// Hands `username` over to `to`, in a request sent to `domain` that
        /// both wallets signed for `domain`.
        async fn hand_over(
            &self,
            server: &TestServer,
            domain: &str,
            to: &Wallet,
            username: &str,
            description: &str,
        ) -> reqwest::Response {
            let timestamp = now_u64();
            let (from_pubkey, to_pubkey) = (self.pubkey.to_string(), to.pubkey.to_string());
            let from_message = signed_message::transfer_from(
                domain,
                username,
                &from_pubkey,
                &to_pubkey,
                timestamp,
            );
            let to_message = signed_message::transfer_to(
                domain,
                username,
                &from_pubkey,
                &to_pubkey,
                description,
                timestamp,
            );
            server
                .request(
                    Method::POST,
                    domain,
                    &format!("/lnurlpay/{to_pubkey}/transfer"),
                )
                .json(&TransferLnurlPayRequest {
                    username: username.to_string(),
                    description: description.to_string(),
                    from_pubkey,
                    from_signature: self.signature(&from_message),
                    to_signature: to.signature(&to_message),
                    timestamp: Some(timestamp),
                })
                .send()
                .await
                .unwrap()
        }

        async fn unregister(
            &self,
            server: &TestServer,
            domain: &str,
            username: &str,
        ) -> reqwest::Response {
            let timestamp = now_u64();
            let message = signed_message::unregister(domain, username, timestamp);
            server
                .request(
                    Method::DELETE,
                    domain,
                    &format!("/lnurlpay/{}", self.pubkey),
                )
                .json(&UnregisterLnurlPayRequest {
                    username: username.to_string(),
                    signature: self.signature(&message),
                    timestamp,
                })
                .send()
                .await
                .unwrap()
        }
    }

    struct ServerWithAliases {
        pool: Pool,
        state: crate::state::State<PostgresRepository>,
        server: TestServer,
        _postgres: ContainerAsync<Postgres>,
    }

    /// The server's own router over a database that allows `tips.example` and
    /// `own.example`, with the aliases the server loads after `sql` ran.
    async fn server_with_aliases(sql: &str) -> ServerWithAliases {
        let (postgres, pool) = test_pool().await;
        let client = pool.get().await.unwrap();
        client
            .batch_execute(
                "INSERT INTO allowed_domains (domain) VALUES ('tips.example'), ('own.example')",
            )
            .await
            .unwrap();
        client.batch_execute(sql).await.unwrap();

        let state = test_state(PostgresRepository::new(pool.clone())).await;
        *state.domains.write().await = crate::domains::DomainMap::from([
            (TARGET.to_string(), None),
            ("own.example".to_string(), None),
        ]);
        refresh_once(&state.db, &state.aliases).await;
        let server = TestServer::start(crate::router(state.clone())).await;
        ServerWithAliases {
            pool,
            state,
            server,
            _postgres: postgres,
        }
    }

    const CASH_IS_AN_ALIAS_OF_TIPS: &str = "INSERT INTO domain_aliases (domain, target_domain)
                                        VALUES ('cash.example', 'tips.example')";

    #[tokio::test]
    async fn a_wallet_registers_through_an_alias() {
        let fixture = server_with_aliases(CASH_IS_AN_ALIAS_OF_TIPS).await;
        let (state, server) = (&fixture.state, &fixture.server);
        let wallet = Wallet::new(1);

        let registered: RegisterLnurlPayResponse = wallet
            .register(server, ALIAS, ALIAS, "alice", "Tips")
            .await
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(registered.lightning_address, "alice@cash.example");
        assert_eq!(registered.lnurl, "lnurlp://cash.example/lnurlp/alice");

        let stored = state.db.get_user_by_name(TARGET, "alice").await.unwrap();
        assert_eq!(
            stored.map(|user| user.pubkey),
            Some(wallet.pubkey.to_string())
        );
        let under_alias = state.db.get_user_by_name(ALIAS, "alice").await.unwrap();
        assert!(under_alias.is_none());

        let recovered = wallet.recover(server, ALIAS).await;
        assert_eq!(recovered.lightning_address, "alice@cash.example");
        assert_eq!(recovered.lnurl, "lnurlp://cash.example/lnurlp/alice");
        assert_eq!(recovered.username, "alice");

        let recovered = wallet.recover(server, TARGET).await;
        assert_eq!(recovered.lightning_address, "alice@tips.example");
        assert_eq!(recovered.lnurl, "lnurlp://tips.example/lnurlp/alice");
    }

    #[tokio::test]
    async fn a_wallet_hands_an_address_over_through_an_alias() {
        let fixture = server_with_aliases(CASH_IS_AN_ALIAS_OF_TIPS).await;
        let (state, server) = (&fixture.state, &fixture.server);
        let (from, to) = (Wallet::new(1), Wallet::new(2));
        from.register(server, TARGET, TARGET, "alice", "Tips")
            .await
            .error_for_status()
            .unwrap();

        let handed_over: TransferLnurlPayResponse = from
            .hand_over(server, ALIAS, &to, "alice", "Tips")
            .await
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(handed_over.lightning_address, "alice@cash.example");

        let stored = state.db.get_user_by_name(TARGET, "alice").await.unwrap();
        assert_eq!(stored.map(|user| user.pubkey), Some(to.pubkey.to_string()));
    }

    #[tokio::test]
    async fn a_wallet_checks_a_username_through_an_alias() {
        let fixture = server_with_aliases(CASH_IS_AN_ALIAS_OF_TIPS).await;
        let server = &fixture.server;
        let wallet = Wallet::new(1);
        Wallet::new(2)
            .register(server, TARGET, TARGET, "alice", "")
            .await
            .error_for_status()
            .unwrap();

        for (username, available) in [("alice", false), ("bob", true)] {
            let timestamp = now_u64();
            let message =
                signed_message::available(ALIAS, &wallet.pubkey.to_string(), username, timestamp);
            let checked: Value = server
                .request(
                    Method::POST,
                    ALIAS,
                    &format!("/lnurlpay/{}/available", wallet.pubkey),
                )
                .json(&CheckUsernameAvailableRequest {
                    username: username.to_string(),
                    signature: wallet.signature(&message),
                    timestamp,
                })
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(checked, json!({ "available": available }));
        }
    }

    #[tokio::test]
    async fn a_signature_has_to_name_the_domain_the_request_was_sent_to() {
        let fixture = server_with_aliases(CASH_IS_AN_ALIAS_OF_TIPS).await;
        let server = &fixture.server;

        let refused = Wallet::new(1)
            .register(server, ALIAS, TARGET, "alice", "")
            .await;
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            refused.json::<Value>().await.unwrap(),
            json!("invalid signature for domain 'cash.example'")
        );
    }

    #[tokio::test]
    async fn a_payer_resolves_an_address_on_both_domains() {
        let fixture = server_with_aliases(CASH_IS_AN_ALIAS_OF_TIPS).await;
        let server = &fixture.server;
        let wallet = Wallet::new(1);
        wallet
            .register(server, TARGET, TARGET, "alice", "")
            .await
            .error_for_status()
            .unwrap();

        let pay = get_json(server, ALIAS, "/.well-known/lnurlp/alice").await;
        assert_eq!(pay["callback"], "https://cash.example/lnurlp/alice/invoice");
        assert_eq!(
            pay["metadata"],
            r#"[["text/plain","Pay to alice@cash.example"],["text/identifier","alice@cash.example"]]"#
        );
        let pay = get_json(server, TARGET, "/lnurlp/alice").await;
        assert_eq!(pay["callback"], "https://tips.example/lnurlp/alice/invoice");
        assert_eq!(
            pay["metadata"],
            r#"[["text/plain","Pay to alice@tips.example"],["text/identifier","alice@tips.example"]]"#
        );

        for domain in [ALIAS, TARGET] {
            let available = get_json(server, domain, "/lnurlpay/available/alice").await;
            assert_eq!(available, json!({ "available": false }));
        }

        let unregistered = wallet.unregister(server, ALIAS, "alice").await;
        assert_eq!(unregistered.status(), StatusCode::OK);
        assert_eq!(unregistered.bytes().await.unwrap().len(), 0);
        for domain in [ALIAS, TARGET] {
            let gone = server
                .request(Method::GET, domain, "/lnurlp/alice")
                .send()
                .await
                .unwrap();
            assert_eq!(gone.status(), StatusCode::NOT_FOUND);
        }
    }

    #[tokio::test]
    async fn an_alias_with_usernames_of_its_own_is_refused() {
        let fixture = server_with_aliases("").await;
        let (state, server) = (&fixture.state, &fixture.server);
        Wallet::new(1)
            .register(server, "own.example", "own.example", "bob", "")
            .await
            .error_for_status()
            .unwrap();
        get_json(server, "own.example", "/lnurlp/bob").await;

        fixture
            .pool
            .get()
            .await
            .unwrap()
            .batch_execute(
                "INSERT INTO domain_aliases (domain, target_domain)
                 VALUES ('own.example', 'tips.example')",
            )
            .await
            .unwrap();
        refresh_once(&state.db, &state.aliases).await;

        assert_eq!(
            state.aliases.read().await["own.example"].refusal,
            Some(Refusal::HasNames)
        );
        let refused = server
            .request(Method::GET, "own.example", "/lnurlp/bob")
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::NOT_FOUND);
        let health = server
            .request(Method::GET, "own.example", "/health")
            .send()
            .await
            .unwrap();
        assert_eq!(health.status(), StatusCode::OK);
        let available = get_json(server, TARGET, "/lnurlpay/available/bob").await;
        assert_eq!(available, json!({ "available": true }));
    }

    #[tokio::test]
    async fn payment_metadata_reaches_a_wallet_as_the_payer_sent_it() {
        let fixture = server_with_aliases(CASH_IS_AN_ALIAS_OF_TIPS).await;
        let (state, server) = (&fixture.state, &fixture.server);
        let wallet = Wallet::new(1);
        let zap_request = r#"{"kind":9734,"tags":[["lnurl","https://tips.example/lnurlp/alice"]]}"#;
        state
            .db
            .upsert_zap(&crate::zap::Zap {
                payment_hash: "00".to_string(),
                zap_request: zap_request.to_string(),
                zap_event: None,
                user_pubkey: wallet.pubkey.to_string(),
                invoice_expiry: i64::MAX,
                updated_at: 1,
                is_user_nostr_key: false,
            })
            .await
            .unwrap();
        state
            .db
            .insert_lnurl_sender_comment(&crate::repository::LnurlSenderComment {
                comment: "for alice@tips.example".to_string(),
                payment_hash: "00".to_string(),
                user_pubkey: wallet.pubkey.to_string(),
                updated_at: 1,
            })
            .await
            .unwrap();

        let timestamp = now_u64();
        let message = signed_message::metadata(ALIAS, &wallet.pubkey.to_string(), timestamp);
        let listed: Value = server
            .request(
                Method::GET,
                ALIAS,
                &format!("/lnurlpay/{}/metadata", wallet.pubkey),
            )
            .header("x-breez-signature", wallet.signature(&message))
            .header("x-breez-timestamp", timestamp.to_string())
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(listed["metadata"][0]["nostr_zap_request"], zap_request);
        assert_eq!(
            listed["metadata"][0]["sender_comment"],
            "for alice@tips.example"
        );
    }
}
