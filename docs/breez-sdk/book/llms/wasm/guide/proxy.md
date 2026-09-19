# Routing traffic through a SOCKS5 proxy

Set `proxy` on the [config](config.md) to send the connections the SDK opens through a SOCKS5 proxy, such as a local Tor daemon.



Set `username` and `password` together for proxies that require RFC 1929 authentication. Setting only one of them is rejected.

## What the proxy covers

Every connection the SDK opens, HTTP and gRPC alike.

Hostnames are resolved **by the proxy**, never locally, so no DNS query reveals which host you are reaching. BIP353 name lookups switch from plain DNS to DNS-over-HTTPS for the same reason: plain DNS is UDP, which a SOCKS5 proxy does not carry, so those queries would otherwise escape the tunnel. They stay DNSSEC-verified either way.

Only the proxy's own address is resolved locally, which is unavoidable: it is the one host that cannot be reached through itself.

## Failing closed

A connection that cannot be established through the proxy fails. The SDK never retries it directly, and setting a proxy also disables system-proxy autodetection, so no environment variable can route traffic around it.

A configuration the SDK cannot honour is rejected where it is supplied, rather than partly applied: at `connect`, or at the constructor of a component built outside the SDK.

| Rejected combination | Why |
|---|---|
| A proxy on WASM | The browser owns connection setup and exposes no proxy control. |
| `proxy` with `connectionsPerOperator` above 1 | Balanced operator connections build their own connectors and cannot be routed. |
| A proxy carrying credentials on `PasskeyConfig` | Nostr relay connections cannot authenticate to a proxy. Wallet labels live on those relays and are one of the salts a wallet seed derives from, so a label that cannot be published cannot be recovered: this fails before a wallet exists rather than after one is funded. |

## Shared SDK Context

A [shared SDK Context](./customizing.md#with-shared-context) owns the pooled HTTP client and gRPC channels, so the proxy has to be set on `SdkContextConfig` as well. It must match the `proxy` on the `Config` of every SDK built from that context: the SDK rejects a mismatch at `connect`, since a disagreement would mean part of the traffic bypassed the proxy.

## Components built outside the SDK

A few APIs run without an SDK instance, so they cannot pick the setting up on their own. Pass the same proxy to each one you use:

- `getSparkStatus`, via `GetSparkStatusRequest`
- `newRestChainService`, via `NewRestChainServiceRequest`
- The Turnkey signer, via `TurnkeyConfig`
- The passkey client, via `PasskeyConfig`. A proxy carrying credentials is rejected when the client is constructed (see above).

A service you supply yourself through `withChainService`, `withFiatService`, `withLnurlClient` or `withLnurlServerClient` already owns its transport, which the SDK cannot inspect or re-route. Make it connect through the proxy yourself; the SDK logs a warning when it sees one alongside a proxy. `withRestChainService` is built on the SDK's own client and is proxied automatically.

## JavaScript and WASM

A SOCKS5 proxy cannot be honoured in a browser, and setting `proxy` on a WASM build is an error rather than a silent direct connection.

In Node, route the SDK by installing a proxy dispatcher on the global `fetch` before connecting, which covers both the HTTP and gRPC calls the WASM build makes:

```javascript
import { setGlobalDispatcher, ProxyAgent } from 'undici'

setGlobalDispatcher(new ProxyAgent('socks5://127.0.0.1:9050'))
```
