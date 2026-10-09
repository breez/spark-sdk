# Running on a server

How the SDK is configured on a server depends on whose wallets it holds and where their keys live.

| | [Multi-user](server_mode.md) | [Treasury](treasury.md) |
|---|---|---|
| Wallets per process | many, one per user | one, owned by the service |
| SDK instance | built per request, disconnected after it | one, long lived |
| Background tasks | off: the host drives sync, claiming and event delivery | on |
| Config preset | `default_server_config` | `default_config` |

**[Multi-user configuration](server_mode.md)** is for a service that holds a wallet per user. The SDK is used as a library: an instance is built for one operation and disconnected, and the host decides when sync, claiming and event delivery happen.

**[Treasury configuration](treasury.md)** is for a single wallet the service itself owns, such as a float or a settlement account. It is a standard deployment with a few defaults worth changing.

A service can run both, as separate SDK instances with separate configs.

## Key management

By default the server holds the wallet's seed. [Using Turnkey](turnkey.md) moves signing into a Turnkey secure enclave: the server holds an API credential instead of key material, and Turnkey policies decide what that credential may sign. It works with either configuration above.

When the server must not be able to send on its own, pair it with [Client signing](client_signing.md): the server prepares and publishes each payment, and the user signs it. With Turnkey, a policy can require the user's own credential for the transfer approval; see [User-approved payments](turnkey.md#user-approved-payments).
