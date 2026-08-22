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

```python
async def getting_started_spark_status():
    try:
        spark_status = await get_spark_status()

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
