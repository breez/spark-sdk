# Listening to events

The SDK emits several events to provide the application with an up-to-date state of the SDK or ongoing payments.

## Event reference

| Event | Payload | What it means and what to do |
| --- | --- | --- |
| {{#enum SdkEvent::Synced}} | none | The wallet finished syncing with the network. Refresh the balance and the payment list. See [getting the SDK info](get_info.md). |
| {{#enum SdkEvent::PaymentSucceeded}} | {{#name Payment}} | A payment completed. The SDK refreshes its cached balance before emitting this, so {{#name get_info}} returns the new value. |
| {{#enum SdkEvent::PaymentPending}} | {{#name Payment}} | A payment is in flight. The same payment is emitted again as succeeded or failed once it settles. |
| {{#enum SdkEvent::PaymentFailed}} | {{#name Payment}} | A payment failed. Its {{#name details}} carry the method-specific context to show the user. |
| {{#enum SdkEvent::NewDeposits}} | {{#name DepositInfo}} list | On-chain deposits were detected. Only deposits whose {{#name is_mature}} is true can be claimed, so show the rest as pending. |
| {{#enum SdkEvent::ClaimedDeposits}} | {{#name DepositInfo}} list | Deposits were claimed into the wallet. The matching payment is emitted separately as {{#enum SdkEvent::PaymentSucceeded}}. |
| {{#enum SdkEvent::UnclaimedDeposits}} | {{#name DepositInfo}} list | The SDK could not claim these. Read {{#name claim_error}} for the reason, then claim manually or refund. See [claiming on-chain deposits](onchain_claims.md). |
| {{#enum SdkEvent::AutoOptimization}} | {{#name AutoOptimizationEvent}} | Progress of the background leaf optimizer. Manual {{#name optimize_leaves}} calls do not emit this. See [custom leaf optimization](optimize.md). |
| {{#enum SdkEvent::LightningAddressChanged}} | {{#name LightningAddressInfo}}, unset when the address was deleted | The Lightning address changed on another device. See [receiving payments using LNURL-Pay](receive_lnurl_pay.md). |
| {{#enum SdkEvent::UnilateralExitStateChanged}} | none | An exit state exported earlier is now out of date. Export it again. See [unilateral exit](unilateral_exit.md). |

The fields of {{#name Payment}} are described in [listing payments](list_payments.md). For
the order in which these events arrive during a receive, see
[receiving payments](receive_payment.md).

### Deposit fields

The three deposit events each carry a list of {{#name DepositInfo}}, whose fields determine
what to do next.

| Field | Meaning |
| --- | --- |
| {{#name txid}}, {{#name vout}} | The on-chain output the deposit came from. |
| {{#name amount_sats}} | Deposit value in satoshis. |
| {{#name is_mature}} | Whether the deposit has enough confirmations to be claimed. |
| {{#name claim_error}} | Why the last claim attempt failed. Set on {{#enum SdkEvent::UnclaimedDeposits}}. |
| {{#name refund_tx}}, {{#name refund_tx_id}} | The refund transaction, once one has been created. |
| {{#name refund_state}} | How far the refund has got towards the network. Read it through {{#name list_unclaimed_deposits}}: a refunded deposit no longer appears in these events. See [tracking a refund](onchain_claims.md#tracking-a-refund). |
| {{#name instant_claim_status}} | State of an instant (0-conf) claim attempt. Unset when none was attempted. |

<h2 id="add-event-listener">
    <a class="header" href="#add-event-listener">Add event listener</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.add_event_listener">API docs</a>
</h2>

{{#tabs getting_started:add-event-listener}}

<h2 id="remove-event-listener">
    <a class="header" href="#remove-event-listener">Remove event listener</a>
    <a class="tag" target="_blank" href="https://breez.github.io/spark-sdk/breez_sdk_spark/struct.BreezSdk.html#method.remove_event_listener">API docs</a>
</h2>

When you no longer need to listen to events, you can remove the listener.

{{#tabs getting_started:remove-event-listener}}
