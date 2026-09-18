# Managing labels

Labels distinguish wallets derived from the same passkey identity. `PasskeyClient.register` and `PasskeyClient.signIn` manage them implicitly, while `PasskeyClient.labels` gives you direct access to the underlying list and publish operations. Both calls prompt the user for a passkey ceremony.

## Listing

Fetch the labels registered for the passkey from Nostr.

```kotlin
val labels = passkey.labels().list()
for (label in labels) {
    // Log.v("Breez", "Found label: $label")
}
```



## Storing

Publish a label to Nostr so it can be discovered later.

```kotlin
passkey.labels().store("personal")
```
