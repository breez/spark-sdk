# Managing labels

Labels distinguish wallets derived from the same passkey identity. `PasskeyClient.register` and `PasskeyClient.sign_in` manage them implicitly, while `PasskeyClient.labels` gives you direct access to the underlying list and publish operations. Both calls prompt the user for a passkey ceremony.

## Listing

Fetch the labels registered for the passkey from Nostr.

```rust
let labels = passkey.labels().list().await?;
for label in &labels {
    println!("Found label: {label}");
}
```



## Storing

Publish a label to Nostr so it can be discovered later.

```rust
passkey.labels().store("personal".to_string()).await?;
```
