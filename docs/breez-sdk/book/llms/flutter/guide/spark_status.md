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

```dart
Future<void> gettingStartedSparkStatus() async {
  final sparkStatus = await getSparkStatus();

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
