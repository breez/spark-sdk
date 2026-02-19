# Enhanced event listeners

The SDK provides typed event listener helpers for Rust consumers that simplify
subscribing to specific event categories. These are convenience wrappers around
the general-purpose {{#name add_event_listener}} method.

> **Note:** The typed helpers (`on_payment`, `on_sync`, `on_deposit`) are
> available in Rust only. For other languages, use {{#name add_event_listener}}
> and match on the event type. See [Listening to events](events.md) for the
> general approach.

## Instant wallet load

When the SDK connects, it automatically spawns a background task that concurrently
fetches the wallet balance and recent payment history. This allows the UI to
display balance and payments quickly without waiting for the full periodic sync.

Each sub-task emits a {{#enum SdkEvent::Synced}} event on completion, so your
sync listener will fire as soon as data is available.

<h2 id="on-payment">
    <a class="header" href="#on-payment">Listening for payments</a>
</h2>

The {{#name on_payment}} helper fires for {{#enum SdkEvent::PaymentSucceeded}},
{{#enum SdkEvent::PaymentPending}}, and {{#enum SdkEvent::PaymentFailed}} events.

{{#tabs events_v2:on-payment}}

<h2 id="on-sync">
    <a class="header" href="#on-sync">Listening for sync events</a>
</h2>

The {{#name on_sync}} helper fires only for {{#enum SdkEvent::Synced}} events.

{{#tabs events_v2:on-sync}}

<h2 id="on-deposit">
    <a class="header" href="#on-deposit">Listening for deposits</a>
</h2>

The {{#name on_deposit}} helper fires for both {{#enum SdkEvent::UnclaimedDeposits}}
and {{#enum SdkEvent::ClaimedDeposits}} events.

{{#tabs events_v2:on-deposit}}

<h2 id="removing-listeners">
    <a class="header" href="#removing-listeners">Removing typed listeners</a>
</h2>

Typed listeners return a listener ID that works with {{#name remove_event_listener}}.

{{#tabs events_v2:remove-typed-listener}}
