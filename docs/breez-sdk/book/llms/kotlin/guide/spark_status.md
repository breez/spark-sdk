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
