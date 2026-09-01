# Spark status

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/fn.get_spark_status.html

The SDK provides a standalone function to check the current operational status of the Spark network. This function does not require an SDK instance and can be called at any time, for example before initializing the SDK.

It returns the overall status of the Spark network, along with a timestamp of when the status was last updated.

The returned `ServiceStatus` has the following values:

- **Operational** - All services are fully operational.
- **Degraded** - Services are experiencing degraded performance.
- **Partial** - Services are partially unavailable.
- **Major** - Services are experiencing a major outage.
- **Unknown** - Service status is unknown.

## Rust

```rust
pub(crate) async fn getting_started_spark_status() -> Result<()> {
    let spark_status = get_spark_status(GetSparkStatusRequest::default()).await?;

    match spark_status.status {
        ServiceStatus::Operational => {
            info!("Spark is fully operational");
        }
        ServiceStatus::Degraded => {
            info!("Spark is experiencing degraded performance");
        }
        ServiceStatus::Partial => {
            info!("Spark is partially unavailable");
        }
        ServiceStatus::Major => {
            info!("Spark is experiencing a major outage");
        }
        ServiceStatus::Unknown => {
            info!("Spark status is unknown");
        }
    }

    info!("Last updated: {}", spark_status.last_updated);
    Ok(())
}
```

## Swift

```swift
func gettingStartedSparkStatus() async throws {
    let sparkStatus = try await getSparkStatus(request: GetSparkStatusRequest())

    switch sparkStatus.status {
    case .operational:
        print("Spark is fully operational")
    case .degraded:
        print("Spark is experiencing degraded performance")
    case .partial:
        print("Spark is partially unavailable")
    case .major:
        print("Spark is experiencing a major outage")
    case .unknown:
        print("Spark status is unknown")
    }

    print("Last updated: \(sparkStatus.lastUpdated)")
}
```

## Kotlin

```kotlin
suspend fun gettingStartedSparkStatus() {
    try {
        val sparkStatus = getSparkStatus(GetSparkStatusRequest())

        when (sparkStatus.status) {
            ServiceStatus.OPERATIONAL -> {
                // Log.v("Breez", "Spark is fully operational")
            }
            ServiceStatus.DEGRADED -> {
                // Log.v("Breez", "Spark is experiencing degraded performance")
            }
            ServiceStatus.PARTIAL -> {
                // Log.v("Breez", "Spark is partially unavailable")
            }
            ServiceStatus.MAJOR -> {
                // Log.v("Breez", "Spark is experiencing a major outage")
            }
            ServiceStatus.UNKNOWN -> {
                // Log.v("Breez", "Spark status is unknown")
            }
        }

        // Log.v("Breez", "Last updated: ${sparkStatus.lastUpdated}")
    } catch (e: Exception) {
        // handle error
    }
}
```

## C#

```csharp
async Task GetSparkStatus()
{
    var sparkStatus = await BreezSdkSparkMethods.GetSparkStatus(
        request: new GetSparkStatusRequest()
    );

    switch (sparkStatus.status)
    {
        case ServiceStatus.Operational:
            Console.WriteLine("Spark is fully operational");
            break;
        case ServiceStatus.Degraded:
            Console.WriteLine("Spark is experiencing degraded performance");
            break;
        case ServiceStatus.Partial:
            Console.WriteLine("Spark is partially unavailable");
            break;
        case ServiceStatus.Major:
            Console.WriteLine("Spark is experiencing a major outage");
            break;
        case ServiceStatus.Unknown:
            Console.WriteLine("Spark status is unknown");
            break;
    }

    Console.WriteLine($"Last updated: {sparkStatus.lastUpdated}");
}
```

## Javascript (Wasm)

```typescript
const sparkStatus = await getSparkStatus()

switch (sparkStatus.status) {
  case 'operational': {
    console.log('Spark is fully operational')
    break
  }
  case 'degraded': {
    console.log('Spark is experiencing degraded performance')
    break
  }
  case 'partial': {
    console.log('Spark is partially unavailable')
    break
  }
  case 'major': {
    console.log('Spark is experiencing a major outage')
    break
  }
  case 'unknown': {
    console.log('Spark status is unknown')
    break
  }
}

console.log(`Last updated: ${sparkStatus.lastUpdated}`)
```

## React Native

```typescript
const sparkStatus = await getSparkStatus({ proxy: undefined })

switch (sparkStatus.status) {
  case ServiceStatus.Operational:
    console.log('Spark is fully operational')
    break
  case ServiceStatus.Degraded:
    console.log('Spark is experiencing degraded performance')
    break
  case ServiceStatus.Partial:
    console.log('Spark is partially unavailable')
    break
  case ServiceStatus.Major:
    console.log('Spark is experiencing a major outage')
    break
  case ServiceStatus.Unknown:
    console.log('Spark status is unknown')
    break
}

console.log(`Last updated: ${sparkStatus.lastUpdated}`)
```

## Flutter

```dart
Future<void> gettingStartedSparkStatus() async {
  final sparkStatus = await getSparkStatus(request: GetSparkStatusRequest());

  switch (sparkStatus.status) {
    case ServiceStatus.operational:
      print("Spark is fully operational");
      break;
    case ServiceStatus.degraded:
      print("Spark is experiencing degraded performance");
      break;
    case ServiceStatus.partial:
      print("Spark is partially unavailable");
      break;
    case ServiceStatus.major:
      print("Spark is experiencing a major outage");
      break;
    case ServiceStatus.unknown:
      print("Spark status is unknown");
      break;
  }

  print("Last updated: ${sparkStatus.lastUpdated}");
}
```

## Python

```python
async def getting_started_spark_status():
    try:
        spark_status = await get_spark_status(request=GetSparkStatusRequest())

        if spark_status.status == ServiceStatus.OPERATIONAL:
            logging.debug("Spark is fully operational")
        elif spark_status.status == ServiceStatus.DEGRADED:
            logging.debug("Spark is experiencing degraded performance")
        elif spark_status.status == ServiceStatus.PARTIAL:
            logging.debug("Spark is partially unavailable")
        elif spark_status.status == ServiceStatus.MAJOR:
            logging.debug("Spark is experiencing a major outage")
        elif spark_status.status == ServiceStatus.UNKNOWN:
            logging.debug("Spark status is unknown")

        logging.debug(f"Last updated: {spark_status.last_updated}")
    except Exception as error:
        logging.error(error)
        raise
```

## Go

```go
func GetSparkStatus() error {
	request := breez_sdk_spark.GetSparkStatusRequest{}
	sparkStatus, err := breez_sdk_spark.GetSparkStatus(request)
	if err != nil {
		return err
	}

	switch sparkStatus.Status {
	case breez_sdk_spark.ServiceStatusOperational:
		log.Printf("Spark is fully operational")
	case breez_sdk_spark.ServiceStatusDegraded:
		log.Printf("Spark is experiencing degraded performance")
	case breez_sdk_spark.ServiceStatusPartial:
		log.Printf("Spark is partially unavailable")
	case breez_sdk_spark.ServiceStatusMajor:
		log.Printf("Spark is experiencing a major outage")
	case breez_sdk_spark.ServiceStatusUnknown:
		log.Printf("Spark status is unknown")
	}

	log.Printf("Last updated: %v", sparkStatus.LastUpdated)
	return nil
}
```

