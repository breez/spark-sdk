# Adding logging

The SDK implements detailed logging via a streaming interface you can manage within your application. The log entries are split into several levels that you can filter and store as desired within your application, for example, by appending them to a log file.

```typescript
class JsLogger {
  log = (l: LogEntry) => {
    console.log(`[${l.level}]: ${l.line}`)
  }
}

const logger = new JsLogger()
initLogging(undefined, logger, undefined)
```
