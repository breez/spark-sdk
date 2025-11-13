<h1 id="lnurl-withdraw">
    <a class="header" href="#lnurl-withdraw">Receiving payments using LNURL-Withdraw</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.lnurl_withdraw">API docs</a>
</h1>

After [parsing](parse.md) an LNURL-Withdraw input, you can use the resulting input data to initiate a withdrawal from an LNURL service.

By default, this function returns immediately. You can override this behavior by specifying a completion timeout in seconds. If the completion timeout is hit, a pending payment object is returned if available. If the payment completes, the completed payment object is returned.

<div class="warning">
<h4>Developer note</h4>
The minimum and maximum withdrawable amount returned from calling parse is denominated in millisatoshi.
</div>

{{#tabs lnurl_withdraw:lnurl-withdraw}}

## Supported Specs

- [LUD-01](https://github.com/lnurl/luds/blob/luds/01.md) LNURL bech32 encoding
- [LUD-03](https://github.com/lnurl/luds/blob/luds/03.md) `withdrawRequest` spec
- [LUD-17](https://github.com/lnurl/luds/blob/luds/17.md) Support for lnurlw prefix with non-bech32-encoded LNURL URLs
