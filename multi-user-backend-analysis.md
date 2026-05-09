# Multi-user backend architecture — summary analysis

This document captures the analysis and decisions agreed so far for adapting
the Breez SDK to a multi-user, server-side deployment model (one process /
pod hosting many tenant wallets behind a shared MySQL or PostgreSQL backend
and shared Spark transports).

The starting reference is the Kotlin/JVM bench harness on the
`daniel-bench-harness` branch (`crates/breez-sdk/breez-bench/kotlin/`),
which validates the per-request `connect → op → disconnect` lifecycle on top
of process-wide shared transports. The analysis below extends that pattern
into a production architecture.

Each topic below is closed: analysis settled, SDK-side changes scoped, and
SSP-side requirements (if any) identified. Implementation is deliberately
out of scope for this document — it captures the planning outcome only.

---

## Topic 1 — Server-mode SDK lifecycle & background work

### Analysis

Today every `connect` spawns:

- A `periodic_sync` `tokio::select!` loop with a 10 s wake-up, a 60 s drift
  sync, and a JWT refresh schedule
  (`crates/breez-sdk/core/src/sdk/sync.rs:30-117`).
- A per-wallet leaf optimizer and a per-wallet token-output optimizer
  (`crates/spark-wallet/src/wallet.rs:2071-2140`).
- One-shot `refresh_leaves`, `refresh_tokens_outputs`,
  `try_recover_lightning_address`, and
  `spawn_spark_private_mode_initialization`
  (`crates/breez-sdk/core/src/sdk/init.rs:55-59`).

None of this composes well at multi-tenant scale:

- In per-request lifecycle the work runs for the duration of a single
  request handler — the periodic loops never tick usefully, the optimizer
  layers passes on top of every send, the one-shot startup work pays the
  full cost on every `connect`.
- In a hypothetical warm-pool lifecycle the same code is worse: N warm
  wallets become N independent loops, all hitting the SSP and operators
  with overlapping work and creating a thundering herd against shared
  transports.

Real-time sync (`PaymentMetadata`, `Contact`, `LightningAddress`
replication for multi-device users) has no consumer in a backend-only
deployment — the backend *is* the storage. It is also documented as
incompatible with shared storage
(`crates/breez-sdk/common/src/sync/background.rs:39-56`); two
`SyncProcessor` instances on the same identity will corrupt revision
tracking against shared MySQL.

**Decision**: in server mode the SDK is treated as a *library*, not an
*agent*. The partner orchestrates everything explicitly. Static-deposit
claim is **webhook-driven via an ephemeral SDK build** (mirroring the
request-handler pattern), with a periodic per-user reconciliation cron as
backstop. There is no global chain watcher and no hidden background work.

### SDK changes

- New flags on `Config`, all defaulting to current behavior (on) for
  client compatibility:
  - `auto_periodic_sync_enabled` — gates the 10 s wake-up arm in
    `periodic_sync`.
  - `auto_lightning_address_recovery` — gates `try_recover_lightning_address`.
  - `private_mode_initialization_on_start` — gates
    `spawn_spark_private_mode_initialization`.
  - `auto_token_optimization_enabled` — gates the token-output
    optimizer.
  - `auto_spark_event_subscription` — gates the per-wallet
    `subscribe_server_events` task. (Flipped off in server mode for
    contract clarity, not for performance — short-lived subscriptions
    over the shared connection are cheap; the work they trigger has no
    consumer in an ephemeral SDK.)
- New `default_server_config(network)` preset that flips all of the
  above off and additionally sets:
  - `real_time_sync_server_url = None`
  - `optimization_config.auto_enabled = false` (existing flag,
    leaf optimizer)
- Hard-error at `SdkBuilder::build()` if a shared MySQL or PostgreSQL
  pool is set *and* `real_time_sync_server_url.is_some()` — the
  combination silently corrupts revision tracking.
- Verify `claim_deposit(txid, vout)` is idempotent (claiming an
  already-claimed UTXO must be a no-op).
- Mirror `default_server_config` and the new flags through bindings per
  the CLAUDE.md "Updating SDK Interfaces" checklist:
  `crates/breez-sdk/wasm/src/models.rs`, `crates/breez-sdk/wasm/src/sdk.rs`,
  `packages/flutter/rust/src/models.rs`, `packages/flutter/rust/src/sdk.rs`.
- Document the partner-side patterns:
  - Webhook handler → ephemeral SDK build → `claim_deposit`.
  - Periodic reconciliation cron → per-user
    `check_and_claim_static_deposits` as backstop for missed/failed
    webhook deliveries.

### Spark required

None for this topic — the existing `SPARK_STATIC_DEPOSIT_FINISHED`
webhook event is sufficient; the webhook event coverage from Topic 2 is
what feeds this design.

---

## Topic 2 — Event delivery (webhooks)

### Analysis

Three candidate channels were evaluated.

1. **Real-time sync stream** (gRPC `Syncer.ListenChanges`). Carries
   metadata records only, not payment events. Per-identity by protocol
   design (the auth signature attests to one pubkey). Documented as
   incompatible with shared storage. Not viable for backend push
   delivery.
2. **Spark events stream** (operator-side gRPC
   `subscribe_to_events`). Carries the right events
   (`ReceiverTransfer`, `Deposit`, `TokenTransaction`) but is
   per-wallet, opens one long-lived stream per identity per coordinator,
   and cannot be batched without a Spark-side protocol change. Even if
   batched, requires the partner to maintain a stateful long-lived
   connection plus replay logic to survive pod restarts.
3. **Webhooks** (SSP-dispatched HTTP POSTs). Decouple event delivery
   from SDK lifecycle entirely (events arrive at an HTTPS endpoint
   regardless of whether any SDK is alive), give server-side queue and
   retry for free, and partners already run HTTPS infrastructure.

**Decision**: webhooks. The remaining design question is registration
shape — per-SDK (today) vs domain-scoped (long-term).

### Phase 1 (start here): per-SDK registration at provisioning

Use the existing `register_webhook(url, secret, event_types)` API
(`crates/breez-sdk/core/src/sdk/api.rs:297`), called once per user at
signup time. The returned `webhook_id` is persisted in the partner DB
keyed by userId; it is never re-listed or re-registered on the request
hot path.

The backend's webhook endpoint must inspect the payload's identity
field (e.g. `receiver_identity_public_key` for Lightning receives) to
route the event to the correct user.

This phase is independent of any Spark-side change to registration shape
and can be implemented as soon as the Spark-side event types listed
below are added.

### Phase 2 (preferred long-term): domain-scoped webhook

A single URL per authenticated partner, with the event payload carrying
`identity_pubkey` for routing — modeled on the existing LNURL webhook
envelope documented in `docs/breez-sdk/src/guide/lnurl_webhooks.md`.

Advantages over phase 1:

- Removes the per-signup SSP round-trip.
- Removes ~100 k webhook entries from SSP-side storage at scale.
- Removes N round-trips on URL rotation (deploy of a new endpoint).
- Removes the per-wallet registration API surface from the partner's
  user-provisioning code.

Phase 2 is deferred until Spark has bandwidth to ship the new
registration shape; the architecture starts on phase 1 to avoid
blocking.

### SDK changes (phase 1)

- Extend `WebhookEventType`
  (`crates/breez-sdk/core/src/models/mod.rs:1788`) with the new event
  variants Spark adds (see below).
- Payload structs for each new event type.
- `From` / `Into` adaptors mapping SSP enums → SDK enums
  (`crates/breez-sdk/core/src/models/adaptors.rs`).
- Public signature-verification helper (HMAC-SHA256 over the raw body,
  comparing against the `X-Spark-Signature` header). Currently each
  partner reimplements this; surface it once in the SDK so the
  contract is consistent across languages.
- Document the partner-side webhook handler shape and the per-event
  idempotency contract (dedup on the event-specific identifier, since
  Spark may retry a non-2xx response).

### SDK changes (phase 2 — deferred)

- Partner-scoped registration API that does not require a per-user
  SDK build to register. Add when Spark ships domain-scoped
  registration.

### Spark required (phase 1)

Add the missing event types to the SSP webhook dispatcher:

- `SPARK_TRANSFER_RECEIVED` — incoming Spark-to-Spark transfer.
- `SPARK_TOKEN_TRANSFER_RECEIVED` — incoming token transfer.
- `BITCOIN_DEPOSIT_CONFIRMED` — confirmation of a deposit to a
  non-static deposit address (only required if this is a distinct path
  from `SPARK_STATIC_DEPOSIT_FINISHED`).
- `SPARK_INVOICE_PAID` — Spark invoice payment received.

The existing HMAC-SHA256 signing and retry/backoff are reused; no
change needed there.

### Spark required (phase 2 — deferred)

- Domain-scoped registration: one URL per authenticated partner,
  payload carries `identity_pubkey`. Modeled on the existing LNURL
  webhook envelope.
