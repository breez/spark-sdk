# Managing labels

Most apps brand a single label and never call these directly. Listing and publishing labels matters when your app supports multiple wallets per passkey identity.

## Listing

Discover labels associated to the passkey using Nostr. {{#name PasskeyClient.sign_in}} already lists labels in discovery mode (when no `label` is specified), so a separate {{#name PasskeyLabels.list}} call (via {{#name PasskeyClient.labels}}) is only needed when re-fetching the label set after sign-in.

{{#tabs passkey:list-labels}}

## Storing

Publish a label to Nostr so it can be discovered later. {{#name PasskeyClient.register}} publishes the label automatically on registration; use {{#name PasskeyLabels.store}} (via {{#name PasskeyClient.labels}}) only when adding a new label to an existing identity (e.g. a "create a new wallet" path on a returning user).

{{#tabs passkey:store-label}}
