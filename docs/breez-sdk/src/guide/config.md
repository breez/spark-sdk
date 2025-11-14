<h1 id="custom-configuration">
    <a class="header" href="#custom-configuration">Custom configuration</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.Config.html">API docs</a>
</h1>

The SDK supports various configuration options to customize its behavior. During [initialization](./initializing.md#basic-initialization), you must provide a configuration object, which we recommend creating by modifying the default configuration. This page describes the available configuration options.

## Max deposit claim fee

Receiving Bitcoin payments through onchain deposits may involve fees. This configuration option controls the automatic claiming of incoming funds, allowing it when the required fees are below specified thresholds (either an absolute fee amount or a feerate). You can also disable automatic claiming entirely. Deposits that are not automatically claimed require manual intervention.

By default, automatic claiming is enabled with a maximum feerate of 1 sat/vB.

More information can be found in the [Handling unclaimed deposits](./unclaimed_deposits.md) page.

{{#tabs config:max-deposit-claim-fee}}

## Synchronization interval

The SDK performs regular background synchronization to check for payment status updates. You can configure how often this synchronization occurs.

The synchronization process is used to detect some payment status updates that are not detected in real-time through event streams.

A shorter synchronization interval provides more responsive detection of payment updates but increases resource usage and may trigger API rate limits. The default interval balances responsiveness with resource efficiency for most use cases.

## LNURL Domain

The LNURL domain to be used for receiving LNURL and Lightning address payments. By default, the [Breez LNURL server](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/lnurl) instance will be used. You may configure a different domain, or set no domain to disable receiving payments using LNURL. For more information, see [Receiving payments using LNURL-Pay](./receive_lnurl_pay.md).

## Prefer Spark over Lightning

An on-off switch that determines whether to prefer settlement using Spark when sending and receiving payments via Lightning invoices. Direct settlement using Spark offers lower fees but reduces privacy.

## External input parsing

The SDK's parsing module can be extended by providing external parsers that are used when input is not recognized. Some [default external parsers](./parse.md#default-external-parsers) are provided but can be disabled. You can add new external parsers as described in [Configuring external parsers](./parse.md#configuring-external-parsers).

## Real-time sync server URL

The SDK synchronizes user data across different SDK instances using a [real-time synchronization server](https://github.com/breez/data-sync). By default, a Breez instance will be used, but you may configure a different instance by providing its URL, or disable it entirely by providing no URL.

## Private mode enabled by default

Configures whether the Spark private mode should be enabled by default. By default, it is enabled. When enabled, the Spark private mode will be enabled on the first initialization of the SDK. If disabled, no changes will be made to the Spark private mode.

{{#tabs config:private-enabled-default}}

<div class="warning">
<h4>Developer note</h4>

This configuration option is only relevant when the SDK is initialized for the first time. To update the user settings after that, or to explicitly disable the Spark private mode, see the [User settings](./user_settings.md) page.

</div>