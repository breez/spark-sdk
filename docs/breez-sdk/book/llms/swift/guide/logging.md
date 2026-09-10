# Adding logging

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/fn.init_logging.html

The SDK implements detailed logging via a streaming interface you can manage within your application. The log entries are split into several levels that you can filter and store as desired within your application, for example, by appending them to a log file.

```swift
class SdkLogger: Logger {
    func log(l: LogEntry) {
        print("Received log [", l.level, "]: ", l.line)
    }
}

func logging() throws {
    try initLogging(logDir: nil, appLogger: SdkLogger(), logFilter: nil)
}
```
