# Adding logging

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/fn.init_logging.html

The SDK implements detailed logging via a streaming interface you can manage within your application. The log entries are split into several levels that you can filter and store as desired within your application, for example, by appending them to a log file.

## Rust

```rust
let data_dir_path = PathBuf::from(&data_dir);
fs::create_dir_all(data_dir_path)?;

init_logging(Some(data_dir), None, None)?;
```

## Swift

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

## Kotlin

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

## C#

```csharp
class SdkLogger : Logger
{
    public void Log(LogEntry l)
    {
        Console.WriteLine($"Received log [{l.level}]: {l.line}");
    }
}

void SetLogger(SdkLogger logger)
{
    BreezSdkSparkMethods.InitLogging(logDir: null, appLogger: logger, logFilter: null);
}
```

## Javascript (Wasm)

```typescript
class JsLogger {
  log = (l: LogEntry) => {
    console.log(`[${l.level}]: ${l.line}`)
  }
}

const logger = new JsLogger()
await initLogging(logger)
```

## React Native

```typescript
class JsLogger {
  log = (l: LogEntry) => {
    console.log(`[${l.level}]: ${l.line}`)
  }
}

const logger = new JsLogger()
initLogging(undefined, logger, undefined)
```

## Flutter

```dart
StreamSubscription<LogEntry>? _logSubscription;
Stream<LogEntry>? _logStream;

// Initializes SDK log stream.
//
// Call once on your Dart entrypoint file, e.g.; `lib/main.dart`
// or singleton SDK service. It is recommended to use a single instance
// of the SDK across your Flutter app.
void initializeLogStream() {
  _logStream ??= initLogging().asBroadcastStream();
}

final _logStreamController = StreamController<LogEntry>.broadcast();
Stream<LogEntry> get logStream => _logStreamController.stream;

// Subscribe to the log stream
void subscribeToLogStream() {
  _logSubscription = _logStream?.listen((logEntry) {
    _logStreamController.add(logEntry);
  }, onError: (e) {
    _logStreamController.addError(e);
  });
}

// Unsubscribe from the log stream
void unsubscribeFromLogStream() {
  _logSubscription?.cancel();
}
```

## Python

```python
class SdkLogger(Logger):
    def log(self, l: LogEntry):
        logging.debug(f"Received log [{l.level}]: {l.line}")


def set_logger(logger: SdkLogger):
    try:
        init_logging(log_dir=None, app_logger=logger, log_filter=None)
    except Exception as error:
        logging.error(error)
        raise
```

## Go

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

