# Managing labels

Labels distinguish wallets derived from the same passkey identity. `PasskeyClient.register` and `PasskeyClient.signIn` manage them implicitly, while `PasskeyClient.labels` gives you direct access to the underlying list and publish operations. Both calls prompt the user for a passkey ceremony.

## Listing

Fetch the labels registered for the passkey from Nostr.

```typescript
const labels = await passkey.labels().list()
for (const label of labels) {
  console.log(`Found label: ${label}`)
}
```



## Storing

Publish a label to Nostr so it can be discovered later.

```typescript
await passkey.labels().store('personal')
```
