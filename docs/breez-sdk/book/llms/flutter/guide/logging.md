# Adding logging

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/fn.init_logging.html

The SDK implements detailed logging via a streaming interface you can manage within your application. The log entries are split into several levels that you can filter and store as desired within your application, for example, by appending them to a log file.

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
