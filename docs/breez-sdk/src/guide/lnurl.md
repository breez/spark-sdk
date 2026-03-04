# Using LNURL and Lightning addresses

The Breez SDK - Spark supports the following <a target="_blank" href="https://github.com/lnurl/luds">LNURL</a> functionality:

- **[Sending payments using LNURL-Pay/Lightning address]** (including BIP353 addresses)
- **[Managing contacts]** — Save frequently used Lightning addresses for quick access
- **[Receiving payments using LNURL-Pay/Lightning address]**
- **[Receiving payments using LNURL-Withdraw]**
- **[Using LNURL-Auth]**
- **[LNURL-Verify]** — Payment verification via [LUD-21](https://github.com/lnurl/luds/blob/luds/21.md) and Nostr Zap receipts via [NIP-57](https://github.com/nostr-protocol/nips/blob/master/57.md). See [configuration](config.md#lnurl-verify-support) to enable.

[Sending payments using LNURL-Pay/Lightning address]: lnurl_pay.md
[Receiving payments using LNURL-Pay/Lightning address]: receive_lnurl_pay.md
[Receiving payments using LNURL-Withdraw]: lnurl_withdraw.md
[Using LNURL-Auth]: lnurl_auth.md
[LNURL-Verify]: receive_lnurl_pay.md#payment-verification-lud-21
[Managing contacts]: contacts.md
