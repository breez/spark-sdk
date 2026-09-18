# Server-side configuration

How the SDK is configured on a server depends on whose wallets it holds.

| | [Multi-user](server_mode.md) | [Treasury](treasury.md) |
|---|---|---|
| Wallets per process | many, one per user | one, owned by the service |
| SDK instance | built per request, disconnected after it | one, long lived |
| Background tasks | off: the host drives sync, claiming and event delivery | on |
| Config preset | {{#name default_server_config}} | {{#name default_config}} |

**[Multi-user configuration](server_mode.md)** is for a service that holds a wallet per user. The SDK is used as a library: an instance is built for one operation and disconnected, and the host decides when sync, claiming and event delivery happen.

**[Treasury configuration](treasury.md)** is for a single wallet the service itself owns, such as a float or a settlement account. It is a standard deployment with a few defaults worth changing.

A service can run both, as separate SDK instances with separate configs.
