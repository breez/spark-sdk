# Customizing the SDK

Using the SDK Builder gives you more control over the initialization and modular components used when the SDK is running. Below you can find examples of initializing the SDK using the SDK Builder and implementing modular components.

The shared-pool, shared-chain-service, and shared-connection-manager components on this page are designed for multi-tenant server deployments — they're most useful in combination with the [Server mode](server_mode.md) SDK profile.

- [Storage](#with-storage) to manage stored data
- [PostgreSQL Backend](#with-postgres-backend) as an alternative storage backend
- [MySQL Backend](#with-mysql-backend) as an alternative storage backend
- [Bitcoin Chain Service](#with-chain-service) to provide network data
- [Shared REST Chain Service](#with-shared-rest-chain-service) to share the chain service HTTP client across SDK instances
- [LNURL Client](#with-lnurl-client) to make REST requests
- [Fiat Service](#with-fiat-service) to provide Fiat currencies and exchange rates
- Change the [Key Set](#with-key-set) to alter the derivation path used
- [Payment Observer](#with-payment-observer) to be notified before payments occur
- [Shared SDK Context](#with-shared-context) to share connection pools and HTTP/gRPC clients across SDK instances

{{#tabs sdk_building:init-sdk-advanced}}

<h2 id="with-storage">
    <a class="header" href="#with-storage">With Storage</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_storage">API docs</a>
</h2>

When using the SDK Builder, you either have to provide a Storage implementation or use the default storage from the SDK.

**Note:** Flutter currently only supports using the default storage.

<h2 id="with-postgres-backend">
    <a class="header" href="#with-postgres-backend">With PostgreSQL Backend</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_storage_backend">API docs</a>
</h2>

The SDK includes a PostgreSQL backend as an alternative to file-based storage. Build a storage config with {{#name postgres_storage}} and pass it to the builder via {{#name with_storage_backend}} — this configures PostgreSQL for all stores (storage, tree store, and token store), which is suitable for server-side deployments with horizontal scaling. To share a single connection pool across multiple SDK instances, supply the same config through a [Shared SDK Context](#with-shared-context); per-tenant scoping (rows isolated by seed identity) is preserved either way.

If your service owns the database schema and applies SDK-compatible migrations externally, set {{#name run_migration}} to `false` on the storage config. The SDK will trust the existing schema and skip all migration runs, including writes to schema migration tables.

**Note:** Not available for React Native or Flutter. For JavaScript/TypeScript, only supported in Node.js (not in the browser).

{{#tabs sdk_building:init-sdk-postgres}}

<div class="warning">
<h4>Developer note</h4>

Sharing the same PostgreSQL database with multiple SDK instances is incompatible with real-time sync. See [Real-time sync server URL](./config.md#real-time-sync-server-url) for how to disable it.

The PostgreSQL tree store can use the same or a separate PostgreSQL database as the PostgreSQL storage. The tree store uses its own set of tables prefixed with `tree_`.

</div>

<h2 id="with-mysql-backend">
    <a class="header" href="#with-mysql-backend">With MySQL Backend</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_storage_backend">API docs</a>
</h2>

The SDK includes a MySQL backend (MySQL 8.0+) as an alternative to file-based storage. Build a storage config with {{#name mysql_storage}} and pass it to the builder via {{#name with_storage_backend}} — this configures MySQL for all stores (storage, tree store, and token store), which is suitable for server-side deployments with horizontal scaling. To share a single connection pool across multiple SDK instances, supply the same config through a [Shared SDK Context](#with-shared-context); per-tenant scoping (rows isolated by seed identity) is preserved either way.

If your service owns the database schema and applies SDK-compatible migrations externally, set {{#name run_migration}} to `false` on the storage config. The SDK will trust the existing schema and skip all migration runs, including writes to schema migration tables.

**Note:** Not available for React Native or Flutter. For JavaScript/TypeScript, only supported in Node.js (not in the browser).

{{#tabs sdk_building:init-sdk-mysql}}

<div class="warning">
<h4>Developer note</h4>

MySQL only accepts URL-form connection strings (`mysql://user:password@host:3306/dbname`); the key=value form supported by PostgreSQL is not available. TLS is enabled by appending `?ssl-mode=required` (or `verify_ca` / `verify_identity`); when using `verify_ca` or `verify_identity` you can supply a custom `root_ca_pem`.

Sharing the same MySQL database with multiple SDK instances is incompatible with real-time sync. See [Real-time sync server URL](./config.md#real-time-sync-server-url) for how to disable it.

The MySQL tree store can use the same or a separate MySQL database as the MySQL storage. The tree store uses its own set of tables prefixed with `tree_`.

</div>

<h2 id="with-chain-service">
    <a class="header" href="#with-chain-service">With Chain Service</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_chain_service">API docs</a>
</h2>

The SDK provides a default Bitcoin Chain Service implementation. If you want to use your own, you can provide it either by using [With REST Chain Service](#with-rest-chain-service) or by implementing the Bitcoin Chain Service interface.

<h2 id="with-rest-chain-service">
    <a class="header" href="#with-rest-chain-service">With REST Chain Service</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_rest_chain_service">API docs</a>
</h2>

The SDK provides a default Bitcoin Chain Service implementation. If you want to use your own, you can provide it either by using [With Chain Service](#with-chain-service) or by providing a URL and optional credentials.

{{#tabs sdk_building:with-rest-chain-service}}

<h2 id="with-shared-rest-chain-service">
    <a class="header" href="#with-shared-rest-chain-service">With Shared REST Chain Service</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/fn.new_rest_chain_service.html">API docs</a>
</h2>

[With REST Chain Service](#with-rest-chain-service) builds a fresh chain service inside each SDK instance. Server processes hosting many wallets at once can share a single REST chain service between every SDK, so they reuse the same pooled HTTP client (and its HTTP/2 connection pool) instead of each opening a fresh one.

Construct one via {{#name new_rest_chain_service}} and pass it to each {{#name SdkBuilder}} via {{#name with_chain_service}}. All SDK instances sharing the chain service must be configured for the same network.

<h2 id="with-fiat-service">
    <a class="header" href="#with-fiat-service">With Fiat Service</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_fiat_service">API docs</a>
</h2>

The SDK by default provides a list of available Fiat currencies and current exchange rates. If you want to use your own, you can provide it by implementing the Fiat Service interface.

<h2 id="with-lnurl-client">
    <a class="header" href="#with-lnurl-client">With LNURL Client</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_lnurl_client">API docs</a>
</h2>

The LNURL Client is used to make REST requests specifically when interacting with LNURL. If you want to use your own, you can it provide by implementing the REST Service interface.

<h2 id="with-key-set">
    <a class="header" href="#with-key-set">With Key Set</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_key_set">API docs</a>
</h2>

The SDK uses by default the Default key set with the account number 1 on Mainnet (0 on Regtest). You can change this to alter the derivation path used with the provided seed:

- **Default** - Uses derivation path `m/8797555'/<account number>` (use address index is ignored)
- **Taproot** - Uses derivation path `m/86'/0'/<account number>'/0/0`<br/>(or `m/86'/0'/0'/0/<account number>` when use address index is enabled)
- **Native Segwit** - Uses derivation path `m/84'/0'/<account number>'/0/0`<br/>(or `m/84'/0'/0'/0/<account number>` when use address index is enabled)
- **Wrapped Segwit** - Uses derivation path `m/49'/0'/<account number>'/0/0`<br/>(or `m/49'/0'/0'/0/<account number>` when use address index is enabled)
- **Legacy** - Uses derivation path `m/44'/0'/<account number>'/0/0`<br/>(or `m/44'/0'/0'/0/<account number>` when use address index is enabled)

{{#tabs sdk_building:with-key-set}}

<h2 id="with-payment-observer">
    <a class="header" href="#with-payment-observer">With Payment Observer</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkBuilder.html#method.with_payment_observer">API docs</a>
</h2>

By implementing the Payment Observer interface you can be notified before a payment is sent. It includes information about the provisional payment including the payment ID, amount to be sent (in satoshis or token base units) and payment details based on the payment method.

**Note:** Flutter currently does not support this.

{{#tabs sdk_building:with-payment-observer}}

<h2 id="with-context">
    <a class="header" href="#with-shared-context">With Shared SDK Context</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.SdkContext.html">API docs</a>
</h2>

An SDK Context bundles every process-shareable resource: the HTTP client (used for SSP GraphQL, chain service and LNURL), the gRPC channels to the Spark operators, the gRPC client to the Breez backend, and — optionally — a PostgreSQL or MySQL connection pool. By default each SDK builds its own. Server processes hosting many wallets at once can construct one SDK Context and pass it to every {{#name SdkBuilder}} so they reuse the same pooled clients instead of each opening fresh ones.

Construct one via {{#name new_shared_sdk_context}} and pass it to each {{#name SdkBuilder}} via {{#name with_shared_context}}. Connections close when the last reference to the SDK Context is dropped; calling {{#name disconnect}} on an SDK instance does not affect them.

The {{#name connections_per_operator}} setting on {{#name SdkContextConfig}} controls how many gRPC connections the context opens to each Spark operator:

- `None` — one connection per operator, multiplexed across every SDK sharing this context. The right choice for almost every deployment.
- `Some(n)` — opens `n` connections per operator and balances requests across them. Worth setting only if the single shared connection has become a bottleneck — for example, latency that climbs with throughput, or operators deployed behind an L7 load balancer where you want client-side fan-out across backend instances.

<div class="warning">
<h4>Developer note</h4>

All SDK instances sharing an SDK Context must be configured for the same network and operator pool. The user agent of the first SDK to construct the context is reused for all subsequent instances.

</div>

### Browser

The SDK Context's gRPC channel pooling is not effective in the browser. Browsers maintain a single HTTP/2 connection per origin and multiplex everything over it; the SDK cannot create or share more.

### Node.js

Node's global `fetch` (undici) negotiates HTTP/2 with the Spark operators automatically and opens additional connections per origin as needed, so most deployments need no tuning. If you do want to cap or expand the per-origin pool, configure undici globally before initialising the SDK:

```js
import { Agent, setGlobalDispatcher } from 'undici'
setGlobalDispatcher(new Agent({ connections: 8 }))
```

This affects every `fetch` in the process, including the SDK's gRPC-web traffic.
