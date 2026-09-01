# Adding logging

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/fn.init_logging.html

The SDK implements detailed logging via a streaming interface you can manage within your application. The log entries are split into several levels that you can filter and store as desired within your application, for example, by appending them to a log file.

```go
type SdkLogger struct{}

func (SdkLogger) Log(l breez_sdk_spark.LogEntry) {
	log.Printf("Received log [%v]: %v", l.Level, l.Line)
}

func SetLogger() {
	var loggerImpl breez_sdk_spark.Logger = SdkLogger{}
	breez_sdk_spark.InitLogging(nil, &loggerImpl, nil)
}
```
