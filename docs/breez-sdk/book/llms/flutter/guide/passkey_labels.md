# Managing labels

Labels distinguish wallets derived from the same passkey identity. `PasskeyClient.register` and `PasskeyClient.signIn` manage them implicitly, while `PasskeyClient.labels` gives you direct access to the underlying list and publish operations. Both calls prompt the user for a passkey ceremony.

## Listing

Fetch the labels registered for the passkey from Nostr.

```dart
final labels = await passkey.labels().list();
for (final label in labels) {
  print("Found label: $label");
}
```



## Storing

Publish a label to Nostr so it can be discovered later.

```dart
await passkey.labels().store(label: "personal");
```
