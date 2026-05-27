<h1 id="server-mode">
    <a class="header" href="#server-mode">Server mode</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/fn.default_server_config.html">API docs</a>
</h1>

Server mode is the SDK profile for **multi-tenant server deployments** where a single process hosts many wallets and builds an ephemeral SDK instance per request. The SDK is treated as a library: the host orchestrates sync, claiming, and event delivery (typically via webhooks) explicitly, so each per-request SDK stays cheap, predictable, and returns fresh state.

Use server mode when:

- You run the SDK behind an HTTP/gRPC service that handles many wallets in the same process.
- Each request builds the SDK, performs one operation, and disconnects.
- Background work that makes sense for a long-lived mobile client (periodic sync, real-time sync, leaf optimization, lightning-address recovery) would be wasted on a per-request lifecycle.

If you're building a mobile or desktop wallet, stay on the default ([client mode](initializing.md)) — server mode disables features your app relies on.

## Selecting server mode

Build the config with {{#name default_server_config}} instead of {{#name default_config}}:

{{#tabs sdk_building:init-sdk-server}}

{{#name default_server_config}} returns the same `Config` as {{#name default_config}} with [{{#name background_tasks_enabled}}](./config.md#background-tasks-enabled) set to `false` and the fields whose background services are gated off — [{{#name real_time_sync_server_url}}](./config.md#real-time-sync-server-url), [{{#name leaf_optimization_config.auto_enabled}}](./config.md#optimization-configuration), and [{{#name token_optimization_config.auto_enabled}}](./config.md#optimization-configuration) — reset to their inactive shape. The SDK rejects builds that leave those fields in their active shape while `background_tasks_enabled` is `false`, so prefer this preset over flipping the flag by hand.

Server mode usually pairs with **shared infrastructure** across SDK instances. See [Customizing the SDK](customizing.md) and the [Shared infrastructure](#shared-infrastructure) section below for the exact wiring.

## What server mode turns off

None of the following per-instance background work is started when {{#name background_tasks_enabled}} is `false`:

- **Periodic sync loop** — the SDK does not auto-sync with the Spark network.
- **Real-time sync client** — no WebSocket subscription to the [real-time sync server](./config.md#real-time-sync-server-url).
- **Spark wallet background processor** — no operator-event subscription, leaf optimizer, or token-output optimizer.
- **Lightning-address recovery** — the SDK does not refresh the registered lightning address on startup.
- **Spark private-mode init** — the [{{#name private_enabled_default}}](./config.md#private-mode-enabled-by-default) preset is **not** applied automatically on first startup; you must opt in once via {{#name update_user_settings}} (see [User settings](user_settings.md)).
- **Flashnet conversion refunder** — no periodic refund pass for failed token conversions.
- **Stable Balance** — Stable Balance is not supported in server mode because its conversion worker is a background service. Do not rely on automatic Bitcoin-to-token conversion on receive, activation/deactivation conversion, or other Stable Balance background behavior in this profile.

Explicit operations such as {{#name sync_wallet}}, {{#name claim_deposit}}, {{#name list_unclaimed_deposits}}, {{#name refund_deposit}}, and {{#name refund_pending_conversions}} continue to work and are the intended entry points in this mode.

## Driving the SDK explicitly

Because nothing runs in the background, the user is responsible for calling the operations that the SDK would otherwise schedule itself. In practice there are only three things to drive, plus one one-time setup per wallet:

### Sync

Call {{#name sync_wallet}} **only when an external event tells you the wallet state has changed**. The two common cases:

1. **A webhook fires for an incoming payment** — a Lightning receive completes, an on-chain deposit confirms, an incoming Spark transfer lands. Run `sync_wallet()` from the webhook handler so the wallet picks up the new state before downstream consumers (balance reads, payment lists, etc.) need it.
2. **You explicitly need to reconcile state** — e.g. a periodic reconciliation job for a specific wallet, or a manual admin action. This is rare in practice; the webhook path covers the steady state.

**Do not** call {{#name sync_wallet}} from user-facing request handlers (e.g. a `GET /balance` endpoint) as a precaution — it's a network round-trip to operators and is not needed if your webhooks are wired up. {{#name get_info}} reads from the local tree store directly and is the right primitive for read paths.

The {{#enum SdkEvent::Synced}} event pattern documented in [Listening to events](events.md) is **not available** in server mode — the SDK has no background subscriber to emit it. Treat {{#name sync_wallet}} as the synchronous primitive instead.

### Claiming on-chain deposits

Server-mode SDKs do not run the periodic deposit detection and claim sweep that the mobile profile uses. When your webhook or chain watcher observes a relevant on-chain deposit, handle it explicitly:

- Call {{#name sync_wallet}} to run the SDK's deposit sync and automatic claim logic using your configured [{{#name max_deposit_claim_fee}}](./config.md#max-deposit-claim-fee).
- If your backend already knows the deposit outpoint and wants to drive a specific claim, call {{#name claim_deposit}} for that `txid`/`vout`.

The standard claim flow documented in [Claiming on-chain deposits](onchain_claims.md) applies.

### Stable Balance

Stable Balance is not available in server mode. The feature depends on the client runtime's background conversion worker, so server-mode SDKs will not automatically convert received Bitcoin to the active stable token and will not process Stable Balance activation/deactivation conversions in the background.

If [{{#name stable_balance_config}}](./config.md#stable-balance-configuration) is set while using server mode, SDK initialization fails with an invalid input error. Explicit token conversion flows used by payment APIs can still be used, but do not configure Stable Balance for a server-mode deployment.

### Token conversion refunds

**Only relevant if your deployment uses [token conversions](token_conversion.md).** If you don't issue or convert tokens, skip this section.

The flashnet conversion refunder doesn't run in the background in server mode. If you do use tokens, your host needs to drive {{#name refund_pending_conversions}} per affected wallet so failed conversions get refunded. A practical pattern is to track which wallets have pending conversions (e.g. by recording them when a conversion fails) and to run the refund pass for just those wallets on a cadence you control — not to spin up an SDK per wallet every minute regardless.

{{#tabs sdk_building:refund-pending-conversions}}

### One-time setup: Spark private mode

The client-mode SDK applies [{{#name private_enabled_default}}](./config.md#private-mode-enabled-by-default) on first startup. Server-mode SDKs do not — each per-request SDK would otherwise pay a redundant storage read to check the flag. At provisioning time (when a new wallet is first registered), call {{#name update_user_settings}} with {{#name spark_private_mode_enabled}} set to `true`. See [User settings](user_settings.md).

## Event delivery via webhooks

Without the background processor, the SDK doesn't emit `PaymentSucceeded` / `PaymentPending` / `ClaimedDeposits` events from operator activity. Deliver those signals through webhooks at your own infrastructure instead:

- [Managing webhooks](webhooks.md) describes the supported event types and registration flow.
- [Lightning Address payment notifications](lnurl_webhooks.md) covers the LNURL server's webhook for incoming LNURL payments.

A typical pipeline: webhook arrives → webhook handler builds a per-request SDK, calls {{#name sync_wallet}} or the relevant explicit operation (e.g. {{#name claim_deposit}}), disconnects.

## Lifecycle pattern

There are three distinct shapes for a server-mode interaction, depending on what triggered it.

### User-facing request handlers

Generate an invoice, send a payment, list history, etc. **Do not call {{#name sync_wallet}} here** — operations that read from local storage ({{#name get_info}}, {{#name list_payments}}, etc.) do not need a defensive sync, and a network round-trip to operators on every request adds latency without changing the answer.

```text
    request in
      ↓
    build SDK (default_server_config + shared infra)
      ↓
    do work (receive_payment / send_payment / list_payments / …)
      ↓
    disconnect()
      ↓
    response out
```

{{#tabs sdk_building:server-mode-request-handler}}

### Webhook handlers and reconciliation jobs

Anything driven by an external signal that the wallet state changed. The exact operation depends on the trigger — they're not chained together in the same handler.

- **Incoming Lightning / Spark transfer webhook** — call {{#name sync_wallet}} so downstream reads see the new payment:

```text
    webhook in  →  build SDK  →  sync_wallet()  →  disconnect()
```

- **On-chain deposit webhook** (or chain watcher) — call {{#name sync_wallet}} to run the deposit sync and automatic claim sweep, or call {{#name claim_deposit}} if you want to claim a known outpoint explicitly:

```text
    webhook in  →  build SDK  →  sync_wallet() / claim_deposit()  →  disconnect()
```

### One-time provisioning

When a wallet is first registered, run a one-time setup pass to apply the configuration the client-mode SDK would otherwise apply itself on first startup — currently the [private mode preset](./config.md#private-mode-enabled-by-default):

```text
    new wallet registered
      ↓
    build SDK (default_server_config + shared infra)
      ↓
    apply one-time user settings
      ↓
    disconnect()
```

{{#tabs sdk_building:server-mode-provisioning}}

### A few notes

- **Building is cheap when infrastructure is shared.** With the shared chain service, MySQL/Postgres pool, and SSP/Connection Managers configured ([see below](#shared-infrastructure)), each per-request SDK reuses HTTP/2 connections, DB pool slots, and gRPC channels — there's no per-request handshake to operators.
- **Always disconnect.** Even though no background loops are running, calling {{#name disconnect}} flushes outstanding storage writes and is the documented lifecycle exit. See [Disconnecting](initializing.md#disconnecting).
- **One SDK per request, not one SDK pinned to a worker thread.** The per-request build is fast enough and avoids cross-tenant state leaks.

<h2 id="shared-infrastructure">
    <a class="header" href="#shared-infrastructure">Shared infrastructure</a>
</h2>

A server-mode deployment normally pairs the profile with shared resources across every per-request SDK. Each of the following is documented in [Customizing the SDK](customizing.md):

- [PostgreSQL Connection Pool](customizing.md#with-postgres-connection-pool) — shared DB pool for storage, tree, and token stores.
- [MySQL Connection Pool](customizing.md#with-mysql-connection-pool) — same for MySQL.
- [Shared REST Chain Service](customizing.md#with-shared-rest-chain-service) — one pooled HTTP client instead of one per SDK.
- [SSP Connection Manager](customizing.md#with-ssp-connection-manager) — share the SSP HTTP client across SDKs.
- [Connection Manager](customizing.md#with-connection-manager) — share gRPC channels to the Spark operators across SDKs.

Pair {{#name default_server_config}} with all of these shared resources — sharing the DB pool, chain service, SSP HTTP client, and gRPC channels across SDKs is the intended deployment shape.

## Driving the {{#name background_tasks_enabled}} field directly

{{#name default_server_config}} is the recommended entry point. If you need to flip the flag on an existing config built another way, see [Background tasks enabled](./config.md#background-tasks-enabled).
