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

```rust
pub(crate) async fn getting_started_spark_status() -> Result<()> {
    let spark_status = get_spark_status().await?;

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
