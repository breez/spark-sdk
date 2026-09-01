# Adding logging

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/fn.init_logging.html

The SDK implements detailed logging via a streaming interface you can manage within your application. The log entries are split into several levels that you can filter and store as desired within your application, for example, by appending them to a log file.

```kotlin
class SdkLogger : Logger {
    override fun log(l: LogEntry) {
        // Log.v("SDKListener", "Received log [${l.level}]: ${l.line}")
    }
}

fun setLogger(logger: SdkLogger) {
    try {
        initLogging(null, logger, null)
    } catch (e: Exception) {
        // handle error
    }
}
```
