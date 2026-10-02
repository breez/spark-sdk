# Custom Lightning address domain

To use Lightning addresses with the Breez SDK, you need a domain served by an LNURL server. You can use the LNURL server hosted by Breez, or run your own.

## Hosted service {#hosted}

Breez runs the LNURL server for your domain. Point your domain to it with a CNAME record, or, if the domain also serves your website, forward the LNURL paths to it from your web server.

### Using a CNAME record {#cname}

This sends all of the domain's web traffic to Breez.

#### Step 1: Add a CNAME record

Add a CNAME record in your domain's DNS settings. You can use your root domain or a subdomain:

| | Root domain | Subdomain |
|---|---|---|
| **Example address** | `user@yourdomain.com` | `user@pay.yourdomain.com` |
| **Host/Name** | `@` | `pay` (or another prefix like `tip` or `donate`) |
| **Type** | CNAME (or ALIAS if available) | CNAME |
| **Value/Target** | `breez.tips` | `breez.tips` |

Some DNS providers do not support CNAME or ALIAS records on the root domain. If yours doesn't, either use a subdomain or configure your domain at the registrar level to use an external DNS provider (like Google Cloud DNS).

> **Note:** If you're using Cloudflare, make sure the CNAME record is set to 'DNS only' (not 'Proxied').

#### Step 2: Register your domain with Breez

[Send us](mailto:contact@breez.technology) your domain name (e.g., yourdomain.com or pay.yourdomain.com), together with the Breez API key you want the LNURL payments on that domain to be associated with.

We will verify and add it to our list of allowed domains.

### Using a reverse proxy {#reverse-proxy}

Your web server forwards the LNURL paths to Breez and serves the rest of the site itself.

#### Step 1: Register your domain with Breez

[Send us](mailto:contact@breez.technology) your domain name and the Breez API key to associate its LNURL payments with, and specify that you will use a reverse proxy. We will verify the domain, add it to our allowed domains, and send you a shared secret for it.

#### Step 2: Forward the LNURL paths to Breez

Forward all requests on these paths to `https://breez.tips` unchanged: any HTTP method, with the path, query string, body and headers as received, apart from the three headers set below. Don't cache the responses.

| Path | Used for |
|---|---|
| `/.well-known/lnurlp/*` | Lightning address lookups |
| `/lnurlp/*` | LNURL-Pay requests, invoice callbacks and payment verification ([LUD-21](https://github.com/lnurl/luds/blob/luds/21.md)) |
| `/lnurlpay/*` | Lightning address registration and management by the SDK |

Set these headers on every forwarded request:

| Header | Value |
|---|---|
| `Host` | Your domain as registered with Breez, in lowercase (not `breez.tips`) |
| `X-Breez-Reverse-Proxy-Shared-Secret` | The shared secret Breez sent you |
| `X-Forwarded-For` | The client's IP address only. Replace any `X-Forwarded-For` the request arrived with. Don't append to it. |

For example, with nginx:

```nginx
location ~ ^/(\.well-known/lnurlp|lnurlp|lnurlpay)/ {
    proxy_pass https://breez.tips;
    proxy_ssl_server_name on;
    proxy_set_header Host yourdomain.com;
    proxy_set_header X-Breez-Reverse-Proxy-Shared-Secret "<your shared secret>";
    proxy_set_header X-Forwarded-For $remote_addr;
}
```

If your web server sits behind a CDN or load balancer, set `X-Forwarded-For` to the client IP that the CDN or load balancer reports instead of `$remote_addr`.

## Self-hosted service {#self-hosted}

Run the [LNURL server](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/lnurl) yourself, point your domain to it, and add the domain to the server's allowed domains. There is no need to contact Breez. See the server's README for how to build, configure and run it.

## Configuring the SDK

Once your domain is set up, pass it as `lnurl_domain` in the SDK configuration, as described in [Configuring Lightning addresses for users](./receive_lnurl_pay.md#configuring-lightning-addresses-for-users).

---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
