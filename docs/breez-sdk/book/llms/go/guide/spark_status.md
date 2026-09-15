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
