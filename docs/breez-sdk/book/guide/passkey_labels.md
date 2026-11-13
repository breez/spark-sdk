# Managing labels

Labels distinguish wallets derived from the same passkey identity. `PasskeyClient.register` and `PasskeyClient.sign_in` manage them implicitly, while `PasskeyClient.labels` gives you direct access to the underlying list and publish operations. Both calls prompt the user for a passkey ceremony.

## Listing

Fetch the labels registered for the passkey from Nostr.

### Rust

```rust
let labels = passkey.labels().list().await?;
for label in &labels {
    println!("Found label: {label}");
}
```

### Swift

```swift
let labels = try await passkey.labels().list()
for label in labels {
    print("Found label: \(label)")
}
```

### Kotlin

```kotlin
val labels = passkey.labels().list()
for (label in labels) {
    // Log.v("Breez", "Found label: $label")
}
```

### C#

```csharp
var labels = await passkey.Labels().List();
foreach (var label in labels)
{
    Console.WriteLine($"Found label: {label}");
}
```

### Javascript (Wasm)

```typescript
const labels = await passkey.labels().list()
for (const label of labels) {
  console.log(`Found label: ${label}`)
}
```

### React Native

```typescript
const labels = await passkey.labels().list()
for (const label of labels) {
  console.log(`Found label: ${label}`)
}
```

### Flutter

```dart
final labels = await passkey.labels().list();
for (final label in labels) {
  print("Found label: $label");
}
```

### Python

```python
labels = await passkey.labels().list()
for label in labels:
    print(f"Found label: {label}")
```

### Go

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

### Rust

```rust
passkey.labels().store("personal".to_string()).await?;
```

### Swift

```swift
try await passkey.labels().store(label: "personal")
```

### Kotlin

```kotlin
passkey.labels().store("personal")
```

### C#

```csharp
await passkey.Labels().Store(label: "personal");
```

### Javascript (Wasm)

```typescript
await passkey.labels().store('personal')
```

### React Native

```typescript
await passkey.labels().store('personal')
```

### Flutter

```dart
await passkey.labels().store(label: "personal");
```

### Python

```python
await passkey.labels().store(label="personal")
```

### Go

```go
err := passkey.Labels().Store("personal")
if err != nil {
	return err
}
```



---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
