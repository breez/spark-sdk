# PRF providers

The built-in `PasskeyProvider` covers the common case. Reach for this page when:

- You need platform-specific provider options (iOS `URLSession` / presentation anchor, Android `Activity` wiring, web `authenticatorAttachment`).
- You're integrating Python, Go, or C# (no built-in `PasskeyProvider` ships for those bindings).
- You need a custom `PrfProvider` (CLI YubiKey, FIDO2, air-gapped backup file, hardware module).

## Built-in PasskeyProvider options

The built-in `PasskeyProvider` takes a `PasskeyProviderOptions`:

| Field | Default | Description |
|--------|---------|-------------|
| `rp_id` | Breez shared RP | Relying Party ID. Your app's domain, or `PasskeyProvider.BREEZ_RP_ID` (`keys.breez.technology`) if Breez-registered. Changing it makes existing passkeys derive a different seed (see [migration considerations](https://github.com/breez/passkey-login/blob/main/SDK%20implementation.md#passkey-migration-considerations)). |
| `rp_name` | `"Breez"` | Display name for your app, shown in some authenticator UIs. Registration-only. |
| `user_name` | `rp_name` | Account identifier shown beneath the display name in the OS picker, e.g. `john@doe.com`. Pass a stable per-user value so each registration is a distinct entry (Apple Passwords dedupes by `(rpId, user.name)`). Registration-only. |
| `user_display_name` | `user_name` | Human-friendly name shown most prominently, e.g. `John Doe`. Registration-only. |

The same `PasskeyProviderOptions` is settable on `passkey_config` via `provider_options`, which builds the provider for you (see [Initialization](./passkey_onboarding.md#initialization)). Construct `PasskeyProvider` directly only for platform-specific options (iOS `URLSession`, web `authenticatorAttachment`) or a custom backend.

**C# / Go / Python limitation**

The SDK does not ship a built-in `PasskeyProvider` for C#, Go, or Python (no native passkey API to wrap). On those bindings, implement your own `PrfProvider` and pass it to `PasskeyClient`.

## Custom PrfProvider

To support a custom authenticator (hardware security key, FIDO2/CTAP2 transport, air-gapped backup file), implement the `PrfProvider` interface directly. The Breez CLI ships [YubiKey](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/yubikey_prf.rs), [FIDO2](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/fido2_prf.rs), and [file-based](https://github.com/breez/spark-sdk/blob/main/crates/breez-sdk/cli/src/passkey/file_prf.rs) implementations as references.

### Rust

```rust
/// Implement PrfProvider for a custom authenticator (hardware key, FIDO2,
/// file-backed). Only derive_seeds and is_supported are required.
struct CustomPrfProvider;

#[async_trait::async_trait]
impl PrfProvider for CustomPrfProvider {
    async fn derive_seeds(
        &self,
        _request: DeriveSeedsRequest,
    ) -> Result<DeriveSeedsOutput, PrfProviderError> {
        // Return one 32-byte PRF output per salt, in input order.
        todo!("Implement using WebAuthn or native passkey APIs")
    }

    async fn is_supported(&self) -> Result<bool, PrfProviderError> {
        todo!("Check platform passkey availability")
    }

    async fn create_passkey(
        &self,
        _exclude_credentials: Vec<Vec<u8>>,
        _salts: Vec<String>,
    ) -> Result<CreatePasskeyOutput, PrfProviderError> {
        // Register a credential and return its ID plus attestation.
        //
        // Return `seeds: None` unless the platform evaluated PRF during the
        // create ceremony and gave one output per salt. Seeds returned here
        // must equal what `derive_seeds` returns for the same salts.
        todo!("Implement registration via WebAuthn create() / native API")
    }

    async fn check_domain_association(&self) -> Result<DomainAssociation, PrfProviderError> {
        // Custom providers without a verification source return Skipped.
        Ok(DomainAssociation::Skipped {
            reason: "CustomPrfProvider does not verify domain association".to_string(),
        })
    }
}
```

### Swift

```swift
// Implement PrfProvider for a custom authenticator (hardware key, FIDO2,
// file-backed). Every method is required: Swift conformance has no
// defaults, unlike the Rust trait.
class CustomPrfProvider: PrfProvider {
    func deriveSeeds(request: DeriveSeedsRequest) async throws -> DeriveSeedsOutput {
        // Return one 32-byte PRF output per salt, in input order.
        fatalError("Implement using WebAuthn or native passkey APIs")
    }

    func isSupported() async throws -> Bool {
        fatalError("Check platform passkey availability")
    }

    func createPasskey(
        excludeCredentials: [Data],
        salts: [String]
    ) async throws -> CreatePasskeyOutput {
        // Register a credential and return its ID plus attestation.
        //
        // Return `seeds: nil` unless the platform evaluated PRF during the
        // create ceremony and gave one output per salt. Seeds returned here
        // must equal what `deriveSeeds` returns for the same salts.
        fatalError("Implement registration via WebAuthn create() / native API")
    }

    func checkDomainAssociation() async throws -> DomainAssociation {
        return .skipped(reason: "CustomPrfProvider does not verify domain association")
    }
}
```

### Kotlin

```kotlin
// Implement PrfProvider for a custom authenticator. Every method is
// required: the generated interface has no defaults, unlike the Rust trait.
class CustomPrfProvider : PrfProvider {
    override suspend fun deriveSeeds(request: DeriveSeedsRequest): DeriveSeedsOutput {
        // Return one 32-byte PRF output per salt, in input order.
        TODO("Implement using WebAuthn or native passkey APIs")
    }

    override suspend fun isSupported(): Boolean {
        TODO("Check platform passkey availability")
    }

    override suspend fun createPasskey(
        excludeCredentials: List<ByteArray>,
        salts: List<String>,
    ): CreatePasskeyOutput {
        // Register a credential and return its ID plus attestation.
        //
        // Return `seeds = null` unless the platform evaluated PRF during the
        // create ceremony and gave one output per salt. Seeds returned here
        // must equal what `deriveSeeds` returns for the same salts.
        TODO("Implement registration via native passkey API")
    }

    override suspend fun checkDomainAssociation(): DomainAssociation {
        return DomainAssociation.Skipped("CustomPrfProvider does not verify domain association")
    }
}
```

### C#

```csharp
// Implement PrfProvider for a custom authenticator (hardware key, FIDO2,
// file-backed). Every method is required: interface members are
// bodiless, unlike the Rust trait's defaults.
class CustomPrfProvider : PrfProvider
{
    public async Task<DeriveSeedsOutput> DeriveSeeds(DeriveSeedsRequest request)
    {
        // Return one 32-byte PRF output per salt, in input order.
        throw new NotImplementedException("Implement using WebAuthn or native passkey APIs");
    }

    public async Task<bool> IsSupported()
    {
        throw new NotImplementedException("Check platform passkey availability");
    }

    public async Task<CreatePasskeyOutput> CreatePasskey(byte[][] excludeCredentials, string[] salts)
    {
        // Register a credential and return its ID plus attestation.
        //
        // Return a null Seeds unless the platform evaluated PRF during the
        // create ceremony and gave one output per salt. Seeds returned here
        // must equal what DeriveSeeds returns for the same salts.
        throw new NotImplementedException("Implement registration via native passkey API");
    }

    public async Task<DomainAssociation> CheckDomainAssociation()
    {
        return await Task.FromResult<DomainAssociation>(
            new DomainAssociation.Skipped("CustomPrfProvider does not verify domain association"));
    }

}
```

### Javascript (Wasm)

```typescript
// Implement PrfProvider for a custom authenticator (hardware key, FIDO2,
// file-backed). Only deriveSeeds and isSupported are required.
class CustomPrfProvider {
  deriveSeeds = async (
    salts: string[]
  ): Promise<{ seeds: Uint8Array[], credentialId: Uint8Array | null }> => {
    // Return one 32-byte PRF output per salt, in input order.
    throw new Error('Implement using WebAuthn or native passkey APIs')
  }

  createPasskey = async (
    _excludeCredentials: Uint8Array[],
    _salts: string[]
  ): Promise<CreatePasskeyOutput> => {
    // Register a credential and return its ID plus attestation.
    //
    // Return `seeds: null` unless the browser evaluated PRF during the
    // create ceremony and gave one output per salt. Seeds returned here
    // must equal what `deriveSeeds` returns for the same salts.
    throw new Error('Implement registration via WebAuthn create() / native API')
  }

  isSupported = async (): Promise<boolean> => {
    throw new Error('Check platform passkey availability')
  }
}
```

### React Native

```typescript
// Implement PrfProvider for a custom authenticator (hardware key, FIDO2,
// file-backed). Every method is required: the generated interface has no
// defaults, unlike the Rust trait.
class CustomPrfProvider {
  deriveSeeds = async (
    _request: { salts: string[] }
  ): Promise<{ seeds: Uint8Array[], credentialId?: Uint8Array }> => {
    // Return one 32-byte PRF output per salt, in input order.
    throw new Error('Implement using WebAuthn or native passkey APIs')
  }

  createPasskey = async (
    _excludeCredentials: Uint8Array[],
    _salts: string[]
  ): Promise<CreatePasskeyOutput> => {
    // Register a credential and return its ID plus attestation.
    //
    // Return `seeds: null` (the field is nullable) unless the platform evaluated PRF during the
    // create ceremony and gave one output per salt. Seeds returned here
    // must equal what `deriveSeeds` returns for the same salts.
    throw new Error('Implement registration via native passkey API')
  }

  isSupported = async (): Promise<boolean> => {
    throw new Error('Check platform passkey availability')
  }
}
```

### Flutter

```dart
// Implement custom callbacks if the built-in PasskeyProvider doesn't
// fit your needs. Pass them to PasskeyClient.fromCallbacks instead
// of going through PasskeyClientBuilder.withPrfProvider.
Future<DeriveSeedsOutput> deriveSeeds(DeriveSeedsRequest request) async {
  // Return one 32-byte PRF output per salt, in input order.
  throw UnimplementedError('Implement using platform passkey APIs');
}

Future<CreatePasskeyOutput> createPasskey(
  List<Uint8List> excludeCredentials,
  List<String> salts,
) async {
  // Register a credential and return its ID plus attestation.
  //
  // Return `seeds` null unless the platform evaluated PRF during the create
  // ceremony and gave one output per salt. Seeds returned here must equal
  // what `deriveSeeds` returns for the same salts.
  throw UnimplementedError('Implement registration via native passkey API');
}

Future<bool> isSupported() async {
  throw UnimplementedError('Check platform passkey availability');
}
```

### Python

```python
# Implement the PrfProvider trait for custom logic if no built-in
# PasskeyProvider ships for your target. Every method is required:
# derive_seeds for derivation, is_supported for the capability probe,
# create_passkey for registration, check_domain_association for the
# advisory RP check.
class CustomPrfProvider(PrfProvider):
    async def derive_seeds(self, request: DeriveSeedsRequest) -> DeriveSeedsOutput:
        # Return one 32-byte PRF output per salt, in input order.
        raise NotImplementedError("Implement using WebAuthn or native passkey APIs")

    async def is_supported(self) -> bool:
        raise NotImplementedError("Check platform passkey availability")

    async def create_passkey(
        self, exclude_credentials: list[bytes], salts: list[str]
    ) -> CreatePasskeyOutput:
        # Register a credential and return its ID plus attestation.
        #
        # Return seeds=None unless the platform evaluated PRF during the create
        # ceremony and gave one output per salt. Seeds returned here must equal
        # what derive_seeds returns for the same salts.
        raise NotImplementedError("Implement registration via native passkey API")

    async def check_domain_association(self) -> DomainAssociation:
        # Optional: verify the app's identity against the platform's
        # domain verification source. Custom providers without a
        # verification source return SKIPPED, which tells callers
        # "proceed with WebAuthn as normal". The UniFFI-generated
        # variant classes are reparented to DomainAssociation at
        # runtime but mypy can't see that, hence the cast.
        return cast(
            DomainAssociation,
            DomainAssociation.SKIPPED(
                reason="CustomPrfProvider does not verify domain association"
            ),
        )
```

### Go

```go
// Implement the PrfProvider interface for a custom authenticator (hardware
// key, FIDO2, file-backed). Every method is required: satisfying a Go
// interface has no defaults, unlike the Rust trait.
type CustomPrfProvider struct{}

func (p *CustomPrfProvider) DeriveSeeds(
	request breez_sdk_spark.DeriveSeedsRequest,
) (breez_sdk_spark.DeriveSeedsOutput, error) {
	// Return one 32-byte PRF output per salt, in input order.
	panic("Implement using WebAuthn or native passkey APIs")
}

func (p *CustomPrfProvider) IsSupported() (bool, error) {
	panic("Check platform passkey availability")
}

func (p *CustomPrfProvider) CreatePasskey(
	excludeCredentials [][]byte,
	salts []string,
) (breez_sdk_spark.CreatePasskeyOutput, error) {
	// Register a credential and return its ID plus attestation.
	//
	// Return a nil Seeds unless the platform evaluated PRF during the create
	// ceremony and gave one output per salt. Seeds returned here must equal
	// what DeriveSeeds returns for the same salts.
	panic("Implement registration via native passkey API")
}

func (p *CustomPrfProvider) CheckDomainAssociation() (breez_sdk_spark.DomainAssociation, error) {
	return breez_sdk_spark.DomainAssociationSkipped{
		Reason: "CustomPrfProvider does not verify domain association",
	}, nil
}
```



---

Identifier casing: `get_info` here is `getInfo` in Swift, Kotlin, JavaScript, React Native and Flutter, and `GetInfo` in Go and C#. Enum variants: `SdkEvent::Synced` is `SdkEvent.SYNCED` in Python, `SdkEvent.synced` in Swift, `SdkEventSynced` in Go, and `SdkEvent.Synced` elsewhere.
