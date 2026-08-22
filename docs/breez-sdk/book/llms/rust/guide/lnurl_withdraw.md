# Receiving payments using LNURL-Withdraw

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.lnurl_withdraw

After [parsing](parse.md) an LNURL-Withdraw input, you can use the resulting input data to initiate a withdrawal from an LNURL service.

By default, this function returns immediately. You can override this behavior by specifying a completion timeout in seconds. If the completion timeout is hit, a pending payment object is returned if available. If the payment completes, the completed payment object is returned.

**Developer note**

The minimum and maximum withdrawable amount returned from calling parse is denominated in millisatoshi.

```rust
// Endpoint can also be of the form:
// lnurlw://domain.com/lnurl-withdraw?key=val
let lnurl_withdraw_url = "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4exctthd96xserjv9mn7um9wdekjmmw843xxwpexdnxzen9vgunsvfexq6rvdecx93rgdmyxcuxverrvcursenpxvukzv3c8qunsdecx33nzwpnvg6ryc3hv93nzvecxgcxgwp3h33lxk";

if let Ok(InputType::LnurlWithdraw(withdraw_request)) = sdk.parse(lnurl_withdraw_url).await {
    // Amount to withdraw in sats between min/max withdrawable amounts
    let amount_sats = 5_000;
    let optional_completion_timeout_secs = Some(30);

    let response = sdk
        .lnurl_withdraw(LnurlWithdrawRequest {
            amount_sats,
            withdraw_request,
            completion_timeout_secs: optional_completion_timeout_secs,
        })
        .await?;

    let payment = response.payment;
    info!("Payment: {payment:?}");
}
```



## Supported Specs

- [LUD-01](https://github.com/lnurl/luds/blob/luds/01.md) LNURL bech32 encoding
- [LUD-03](https://github.com/lnurl/luds/blob/luds/03.md) `withdrawRequest` spec
- [LUD-17](https://github.com/lnurl/luds/blob/luds/17.md) Support for lnurlw prefix with non-bech32-encoded LNURL URLs
