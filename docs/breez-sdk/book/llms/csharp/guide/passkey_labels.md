# Managing labels

Labels distinguish wallets derived from the same passkey identity. `PasskeyClient.Register` and `PasskeyClient.SignIn` manage them implicitly, while `PasskeyClient.Labels` gives you direct access to the underlying list and publish operations. Both calls prompt the user for a passkey ceremony.

## Listing

Fetch the labels registered for the passkey from Nostr.

```csharp
var labels = await passkey.Labels().List();
foreach (var label in labels)
{
    Console.WriteLine($"Found label: {label}");
}
```



## Storing

Publish a label to Nostr so it can be discovered later.

```csharp
await passkey.Labels().Store(label: "personal");
```
