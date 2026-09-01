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
