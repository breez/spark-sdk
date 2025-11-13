<h1 id="lnurl-pay">
    <a class="header" href="#lnurl-pay">Sending payments using LNURL-Pay and Lightning address</a>
</h1>

<h2 id="preparing-lnurl-payments">
    <a class="header" href="#preparing-lnurl-payments">Preparing LNURL Payments</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.prepare_lnurl_pay">API docs</a>
</h2>

During the prepare step, the SDK ensures that the inputs are valid with respect to the LNURL-pay request,
and also returns the fees related to the payment so they can be confirmed.

{{#tabs lnurl_pay:prepare-lnurl-pay}}

<h2 id="lnurl-payments">
    <a class="header" href="#lnurl-payments">LNURL Payments</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.lnurl_pay">API docs</a>
</h2>

Once the payment has been prepared and the fees are accepted, all you have to do is pass the prepare response as an argument to the
LNURL pay method.

{{#tabs lnurl_pay:lnurl-pay}}

<div class="warning">
<h4>Developer note</h4>
By default when the LNURL-pay results in a success action with a URL, the URL is validated to check if there is a mismatch with the LNURL callback domain. You can disable this behaviour by setting the optional validation <code>PrepareLnurlPayRequest</code> param to false.
</div>

## Supported Specs

- [LUD-01](https://github.com/lnurl/luds/blob/luds/01.md) LNURL bech32 encoding
- [LUD-06](https://github.com/lnurl/luds/blob/luds/06.md) `payRequest` spec
- [LUD-09](https://github.com/lnurl/luds/blob/luds/09.md) `successAction` field for `payRequest`
- [LUD-16](https://github.com/lnurl/luds/blob/luds/16.md) LN Address
- [LUD-17](https://github.com/lnurl/luds/blob/luds/17.md) Support for lnurlp prefix with non-bech32-encoded LNURL URLs
