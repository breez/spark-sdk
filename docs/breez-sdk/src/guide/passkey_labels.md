# Managing labels

Labels distinguish wallets derived from the same passkey identity. {{#name PasskeyClient.register}} and {{#name PasskeyClient.sign_in}} manage them implicitly, while {{#name PasskeyClient.labels}} gives you direct access to the underlying list and publish operations. Both calls prompt the user for a passkey ceremony.

## Listing

Fetch the labels registered for the passkey from Nostr.

{{#tabs passkey:list-labels}}

## Storing

Publish a label to Nostr so it can be discovered later.

{{#tabs passkey:store-label}}
