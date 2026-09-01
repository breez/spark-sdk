# Managing labels

Labels distinguish wallets derived from the same passkey identity. `PasskeyClient.Register` and `PasskeyClient.SignIn` manage them implicitly, while `PasskeyClient.Labels` gives you direct access to the underlying list and publish operations. Both calls prompt the user for a passkey ceremony.

## Listing

Fetch the labels registered for the passkey from Nostr.

```go
labels, err := passkey.Labels().List()
if err != nil {
	return nil, err
}
for _, label := range labels {
	log.Printf("Found label: %s", label)
}
```



## Storing

Publish a label to Nostr so it can be discovered later.

```go
err := passkey.Labels().Store("personal")
if err != nil {
	return err
}
```
