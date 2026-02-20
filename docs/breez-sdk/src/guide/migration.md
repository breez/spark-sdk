# Migration Guide

This guide covers breaking changes and deprecations across SDK versions, with migration steps for each platform.

## 0.10.0: Namespace and Client Type

Free functions have been replaced by methods on the `BreezSdkSpark` struct, and the client type has been renamed from `BreezSdk` to `BreezSparkClient`.

### What Changed

| Before (deprecated) | After |
|---------------------|-------|
| `defaultConfig(network)` | `BreezSdkSpark.defaultConfig(network)` |
| `connect(request)` | `BreezSdkSpark.connect(request)` |
| `parse(input)` | `BreezSdkSpark.parse(input)` |
| `initLogging(...)` | `BreezSdkSpark.initLogging(...)` |
| `connectWithSigner(request)` | `BreezSdkSpark.connectWithSigner(request)` |
| `defaultExternalSigner(...)` | `BreezSdkSpark.defaultExternalSigner(...)` |
| `getSparkStatus()` | `BreezSdkSpark.getSparkStatus()` |
| `BreezSdk` (client type) | `BreezSparkClient` |

Instance methods on the client (`getInfo`, `sendPayment`, `disconnect`, etc.) are **unchanged**.

---

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

#### Before
```rust,ignore
use breez_sdk_spark::*;

let config = default_config(Network::Mainnet);
let sdk: BreezSdk = connect(ConnectRequest { config, seed, storage_dir }).await?;

init_logging(Some(data_dir), None, None)?;
let status = get_spark_status().await?;
```

#### After
```rust,ignore
use breez_sdk_spark::*;

let config = BreezSdkSpark::default_config(Network::Mainnet);
let client: BreezSparkClient = BreezSdkSpark::connect(ConnectRequest { config, seed, storage_dir }).await?;

BreezSdkSpark::init_logging(Some(data_dir), None, None)?;
let status = BreezSdkSpark::get_spark_status().await?;
```

</section>

<div slot="title">Swift</div>
<section>

No code changes required. Swift uses the module name `BreezSdkSpark` as the namespace automatically:

```swift,ignore
// Both forms work — the module prefix is implicit
let config = defaultConfig(network: .mainnet)
let config = BreezSdkSpark.defaultConfig(network: .mainnet)
```

</section>

<div slot="title">Kotlin</div>
<section>

#### Before
```kotlin,ignore
import breez_sdk_spark.*

val config = defaultConfig(Network.MAINNET)
val sdk: BreezSdk = connect(ConnectRequest(config, seed, storageDir))
```

#### After
```kotlin,ignore
import breez_sdk_spark.BreezSdkSpark
import breez_sdk_spark.BreezSparkClient

val config = BreezSdkSpark.defaultConfig(Network.MAINNET)
val client: BreezSparkClient = BreezSdkSpark.connect(ConnectRequest(config, seed, storageDir))
```

</section>

<div slot="title">C#</div>
<section>

#### Before
```csharp,ignore
using Breez.Sdk.Spark;

var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet);
var sdk = await BreezSdkSparkMethods.Connect(new ConnectRequest(config, seed, storageDir));
```

#### After
```csharp,ignore
using Breez.Sdk.Spark;

var config = BreezSdkSpark.DefaultConfig(Network.Mainnet);
var client = await BreezSdkSpark.Connect(new ConnectRequest(config, seed, storageDir));
```

</section>

<div slot="title">Javascript</div>
<section>

#### Before
```typescript,ignore
import { connect, defaultConfig } from '@breeztech/breez-sdk-spark'

const config = defaultConfig('mainnet')
const sdk = await connect({ config, seed, storageDir: './.data' })
```

#### After
```typescript,ignore
import { BreezSdkSpark } from '@breeztech/breez-sdk-spark'

const config = BreezSdkSpark.defaultConfig('mainnet')
const client = await BreezSdkSpark.connect({ config, seed, storageDir: './.data' })
```

</section>

<div slot="title">React Native</div>
<section>

#### Before
```typescript,ignore
import { connect, defaultConfig } from '@breeztech/breez-sdk-spark-react-native'

const config = defaultConfig('mainnet')
const sdk = await connect({ config, seed, storageDir })
```

#### After
```typescript,ignore
import { BreezSdkSpark } from '@breeztech/breez-sdk-spark-react-native'

const config = BreezSdkSpark.defaultConfig('mainnet')
const client = await BreezSdkSpark.connect({ config, seed, storageDir })
```

</section>

<div slot="title">Flutter</div>
<section>

#### Before
```dart,ignore
import 'package:breez_sdk_spark/breez_sdk_spark.dart';

final config = defaultConfig(network: Network.mainnet);
final sdk = await connect(request: ConnectRequest(config: config, seed: seed, storageDir: storageDir));
```

#### After
```dart,ignore
import 'package:breez_sdk_spark/breez_sdk_spark.dart';

final config = BreezSdkSpark.defaultConfig(network: Network.mainnet);
final client = await BreezSdkSpark.connect(request: ConnectRequest(config: config, seed: seed, storageDir: storageDir));
```

</section>

<div slot="title">Python</div>
<section>

#### Before
```python,ignore
from breez_sdk_spark import default_config, connect, Network

config = default_config(Network.MAINNET)
sdk = await connect(ConnectRequest(config, seed, storage_dir))
```

#### After
```python,ignore
from breez_sdk_spark import BreezSdkSpark, BreezSparkClient, Network

config = BreezSdkSpark.default_config(Network.MAINNET)
client: BreezSparkClient = await BreezSdkSpark.connect(ConnectRequest(config, seed, storage_dir))
```

</section>

<div slot="title">Go</div>
<section>

No code changes required. Go uses the package name as the namespace automatically:

```go,ignore
// Both forms are identical — Go always uses the package prefix
config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
client, err := breez_sdk_spark.Connect(breez_sdk_spark.ConnectRequest{...})
```

</section>

</custom-tabs>
