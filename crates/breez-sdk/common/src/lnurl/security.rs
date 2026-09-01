//! Validation of server-chosen LNURL destinations (SSRF hardening).
//!
//! The LNURL protocol has the wallet fetch URLs chosen by the remote service
//! (pay and withdraw callbacks). Left unconstrained, a malicious service can
//! point those at loopback, LAN or cloud-metadata addresses, or downgrade to
//! plaintext http. This module constrains them: callbacks from a public
//! endpoint must be https to a public host, while an endpoint the user
//! deliberately chose on loopback or Tor (dev setups) keeps working.

use std::net::{Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

use crate::lnurl::error::{LnurlError, LnurlResult};

/// How far the flow trusts a server-chosen callback, derived from where the
/// user-entered endpoint itself lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackTrust {
    /// Public endpoint: callbacks must be https to a public host.
    Strict,
    /// Endpoint on a `.onion` host: still a remote party, so callbacks keep
    /// the public-host rules, except that a `.onion` callback may use http
    /// (Tor already encrypts the transport).
    Onion,
    /// The user chose an endpoint on their own machine (dev setups), so its
    /// callbacks are exempt from the public-host rules.
    Loopback,
}

/// Derives the [`CallbackTrust`] for callbacks served by `endpoint_url`, the
/// URL the LNURL flow started from. An unparsable or empty URL is `Strict`.
pub fn callback_trust(endpoint_url: &str) -> CallbackTrust {
    let Ok(url) = Url::parse(endpoint_url) else {
        return CallbackTrust::Strict;
    };
    match url.host() {
        Some(Host::Domain(domain)) if is_or_subdomain_of(domain, "localhost") => {
            CallbackTrust::Loopback
        }
        Some(Host::Domain(domain)) if is_or_subdomain_of(domain, "onion") => CallbackTrust::Onion,
        Some(Host::Ipv4(ip)) if ip.is_loopback() => CallbackTrust::Loopback,
        Some(Host::Ipv6(ip)) if ip.is_loopback() => CallbackTrust::Loopback,
        _ => CallbackTrust::Strict,
    }
}

/// Validates a server-chosen callback URL before it is fetched.
///
/// Under [`CallbackTrust::Strict`] the URL must be https, carry no userinfo
/// credentials, and not target a loopback, private, link-local or otherwise
/// reserved host. [`CallbackTrust::Onion`] additionally accepts an http
/// callback when it targets a `.onion` host. Under [`CallbackTrust::Loopback`]
/// the URL only has to parse.
pub fn validate_callback_url(callback: &str, trust: CallbackTrust) -> LnurlResult<Url> {
    let url = Url::parse(callback).map_err(|_| LnurlError::invalid_uri("invalid callback uri"))?;
    if trust == CallbackTrust::Loopback {
        return Ok(url);
    }
    let onion_callback = matches!(
        url.host(),
        Some(Host::Domain(domain)) if is_or_subdomain_of(domain, "onion")
    );
    let http_allowed = trust == CallbackTrust::Onion && onion_callback;
    if url.scheme() != "https" && !(url.scheme() == "http" && http_allowed) {
        return Err(LnurlError::invalid_uri("callback must use https"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(LnurlError::invalid_uri(
            "callback must not carry credentials",
        ));
    }
    match url.host() {
        None => return Err(LnurlError::invalid_uri("callback has no host")),
        Some(Host::Ipv4(ip)) if is_reserved_ipv4(ip) => {
            return Err(LnurlError::invalid_uri(
                "callback targets a reserved address",
            ));
        }
        Some(Host::Ipv6(ip)) if is_reserved_ipv6(ip) => {
            return Err(LnurlError::invalid_uri(
                "callback targets a reserved address",
            ));
        }
        Some(Host::Domain(domain)) => {
            // `local` is mDNS and single-label names are resolvable only on
            // the local network: a public endpoint has no business sending the
            // wallet there.
            let normalized = domain.strip_suffix('.').unwrap_or(domain);
            if is_or_subdomain_of(domain, "localhost")
                || is_or_subdomain_of(domain, "local")
                || !normalized.contains('.')
            {
                return Err(LnurlError::invalid_uri("callback targets a local hostname"));
            }
        }
        Some(_) => {}
    }
    Ok(url)
}

/// Resolves the URL's host and rejects it if any answer is a reserved
/// address, closing the "public hostname, private A record" hole that
/// [`validate_callback_url`] cannot see. Literal-IP hosts are already
/// covered there, so they pass through.
///
/// The address the later connection uses may differ from the answers checked
/// here (DNS rebinding); that residual window is accepted because the
/// https-only rule means a private target must also present a certificate
/// valid for the attacker's hostname to receive any request bytes.
///
/// Callers must skip this when the traffic is proxied: the lookup would run
/// outside the proxy, leaking hostnames (and the proxy's network is out of
/// this trust model anyway).
#[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
pub async fn ensure_host_resolves_public(url: &Url) -> LnurlResult<()> {
    let Some(Host::Domain(domain)) = url.host() else {
        return Ok(());
    };
    // `.onion` names are not DNS-resolvable and must not be looked up
    // (RFC 7686); they only connect through a proxy that understands them.
    if is_or_subdomain_of(domain, "onion") {
        return Ok(());
    }
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs = tokio::net::lookup_host((domain, port))
        .await
        .map_err(|e| LnurlError::general(format!("callback host lookup failed: {e}")))?;
    for addr in addrs {
        let reserved = match addr.ip() {
            std::net::IpAddr::V4(ip) => is_reserved_ipv4(ip),
            std::net::IpAddr::V6(ip) => is_reserved_ipv6(ip),
        };
        if reserved {
            return Err(LnurlError::invalid_uri(
                "callback host resolves to a reserved address",
            ));
        }
    }
    Ok(())
}

/// Redirect filter for LNURL traffic: every hop must satisfy
/// [`validate_callback_url`] under the trust derived from the original
/// request URL, so a redirect cannot reach a destination the original URL
/// could not have named. Hops get no DNS preflight (the redirect policy is
/// synchronous); that residual is the same accepted rebinding window, and
/// the https requirement still gates what a private target can receive.
pub fn lnurl_redirect_filter() -> platform_utils::RedirectFilter {
    std::sync::Arc::new(|next, original| {
        let trust = callback_trust(original.as_str());
        validate_callback_url(next.as_str(), trust)
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
}

/// True when `domain`'s last DNS label is `label` (so `label` itself or any
/// subdomain of it). Compares ASCII case-insensitively and tolerates the
/// absolute-form trailing dot, so it holds for any host string as-is.
fn is_or_subdomain_of(domain: &str, label: &str) -> bool {
    let domain = domain.strip_suffix('.').unwrap_or(domain);
    domain
        .rsplit('.')
        .next()
        .is_some_and(|last| last.eq_ignore_ascii_case(label))
}

/// IPv4 ranges a server-chosen destination must never target: loopback,
/// private, link-local, CGNAT, benchmarking, documentation, multicast,
/// unspecified, broadcast and the 240/4 reserved block.
fn is_reserved_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_documentation()
        // 100.64.0.0/10 (CGNAT), no stable std helper
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        // 192.0.0.0/24 (IETF protocol assignments)
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        // 198.18.0.0/15 (benchmarking)
        || (octets[0] == 198 && (18..=19).contains(&octets[1]))
        // 240.0.0.0/4 (reserved)
        || octets[0] >= 240
}

/// IPv6 equivalent of [`is_reserved_ipv4`], including IPv4-mapped forms and
/// the NAT64 and documentation prefixes.
fn is_reserved_ipv6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_reserved_ipv4(v4);
    }
    // Deprecated IPv4-compatible form (::a.b.c.d); `to_ipv4` also maps `::1`
    // and `::`, both already rejected below, so only real v4 forms matter.
    if let Some(v4) = ip.to_ipv4()
        && !ip.is_loopback()
        && !ip.is_unspecified()
    {
        return is_reserved_ipv4(v4);
    }
    let segments = ip.segments();
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        // 64:ff9b::/96 (NAT64): embeds an IPv4 address reachable through a
        // translator, so check the embedded address
        || (segments[0] == 0x64
            && segments[1] == 0xff9b
            && is_reserved_ipv4(Ipv4Addr::new(
                (segments[6] >> 8) as u8,
                (segments[6] & 0xff) as u8,
                (segments[7] >> 8) as u8,
                (segments[7] & 0xff) as u8,
            )))
        // 2001:db8::/32 (documentation)
        || (segments[0] == 0x2001 && segments[1] == 0xdb8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    use macros::test_all;

    #[test_all]
    fn trust_is_derived_from_the_endpoint_host() {
        for url in [
            "http://localhost/lnurlp/x",
            "http://localhost:8080/lnurlp/x",
            "http://127.0.0.1:8080/lnurlp/x",
            "http://[::1]/lnurlp/x",
            "http://dev.localhost/lnurlp/x",
        ] {
            assert_eq!(callback_trust(url), CallbackTrust::Loopback, "{url}");
        }
        assert_eq!(
            callback_trust("http://abcdef.onion/lnurlp/x"),
            CallbackTrust::Onion
        );
        for url in [
            "https://service.com/lnurlp/x",
            // Prefix tricks must not unlock loopback or onion trust
            "https://localhost.evil.com/lnurlp/x",
            "https://127.0.0.1.evil.com/lnurlp/x",
            "https://onion.evil.com/lnurlp/x",
            "",
            "not a url",
        ] {
            assert_eq!(callback_trust(url), CallbackTrust::Strict, "{url}");
        }
    }

    /// An onion endpoint is a remote party: its callbacks keep the strict
    /// rules, except that a `.onion` callback may use http.
    #[test_all]
    fn onion_trust_keeps_public_host_rules() {
        for callback in [
            "http://abcdef.onion/cb",
            "https://abcdef.onion/cb",
            "https://service.com/cb",
        ] {
            assert!(
                validate_callback_url(callback, CallbackTrust::Onion).is_ok(),
                "{callback}"
            );
        }
        for callback in [
            "http://service.com/cb",
            "http://127.0.0.1:8332/cb",
            "https://127.0.0.1:8332/cb",
            "https://192.168.1.1/cb",
            "https://169.254.169.254/latest/meta-data/",
            "http://localhost/cb",
            "https://user:pass@abcdef.onion/cb",
        ] {
            assert!(
                validate_callback_url(callback, CallbackTrust::Onion).is_err(),
                "{callback}"
            );
        }
    }

    #[test_all]
    fn strict_rejects_non_https_and_credentials() {
        for callback in [
            "http://service.com/cb",
            "ftp://service.com/cb",
            "https://user@service.com/cb",
            "https://user:pass@service.com/cb",
        ] {
            assert!(
                validate_callback_url(callback, CallbackTrust::Strict).is_err(),
                "{callback}"
            );
        }
    }

    #[test_all]
    fn strict_rejects_reserved_literal_hosts() {
        for callback in [
            "https://127.0.0.1:8332/cb",
            "https://10.0.0.1/cb",
            "https://172.16.0.1/cb",
            "https://192.168.1.1/cb",
            "https://169.254.169.254/latest/meta-data/",
            "https://100.64.0.1/cb",
            "https://192.0.0.192/cb",
            "https://198.18.0.1/cb",
            "https://240.0.0.1/cb",
            "https://0.0.0.0/cb",
            "https://255.255.255.255/cb",
            // WHATWG integer and octal host forms normalize to 127.0.0.1
            "https://2130706433/cb",
            "https://0x7f000001/cb",
            "https://017700000001/cb",
            "https://[::1]/cb",
            "https://[::]/cb",
            "https://[::ffff:127.0.0.1]/cb",
            "https://[::ffff:10.0.0.1]/cb",
            "https://[::127.0.0.1]/cb",
            "https://[fd00::1]/cb",
            "https://[fe80::1]/cb",
            "https://[ff02::1]/cb",
            "https://[64:ff9b::7f00:1]/cb",
            "https://[2001:db8::1]/cb",
            "https://localhost/cb",
            "https://LOCALHOST/cb",
            "https://localhost./cb",
            "https://dev.localhost/cb",
            "https://printer.local/cb",
            "https://intranet/cb",
            "https://intranet./cb",
        ] {
            assert!(
                validate_callback_url(callback, CallbackTrust::Strict).is_err(),
                "{callback}"
            );
        }
    }

    #[test_all]
    fn strict_accepts_public_https() {
        for callback in [
            "https://service.com/cb?k=v",
            "https://sub.service.com:8443/cb",
            "https://93.184.216.34/cb",
            "https://[2607:f8b0::1]/cb",
        ] {
            assert!(
                validate_callback_url(callback, CallbackTrust::Strict).is_ok(),
                "{callback}"
            );
        }
    }

    #[test_all]
    fn loopback_trust_allows_dev_callbacks() {
        for callback in [
            "http://127.0.0.1:8080/cb",
            "http://localhost:8080/cb",
            "http://abcdef.onion/cb",
        ] {
            assert!(
                validate_callback_url(callback, CallbackTrust::Loopback).is_ok(),
                "{callback}"
            );
        }
    }

    #[test_all]
    fn redirect_filter_applies_the_original_urls_trust() {
        let filter = lnurl_redirect_filter();
        let u = |s: &str| Url::parse(s).unwrap();

        // Strict original: https public hops only
        let public = u("https://service.com/cb");
        assert!(filter(&u("https://other.com/cb"), &public).is_ok());
        assert!(filter(&u("http://other.com/cb"), &public).is_err());
        assert!(filter(&u("https://127.0.0.1:8332/cb"), &public).is_err());
        assert!(filter(&u("https://192.168.1.1/cb"), &public).is_err());

        // Loopback original (dev): hops unrestricted
        let dev = u("http://127.0.0.1:8080/cb");
        assert!(filter(&u("http://127.0.0.1:8081/cb"), &dev).is_ok());

        // Onion original: onion hops may stay http, private targets refused
        let onion = u("http://abcdef.onion/cb");
        assert!(filter(&u("http://ghijkl.onion/cb"), &onion).is_ok());
        assert!(filter(&u("http://127.0.0.1:8332/cb"), &onion).is_err());
    }

    #[cfg(not(all(target_family = "wasm", target_os = "unknown")))]
    #[tokio::test]
    async fn preflight_skips_onion_hosts() {
        let url = Url::parse("http://abcdef.onion/cb").unwrap();
        assert!(ensure_host_resolves_public(&url).await.is_ok());
    }
}
