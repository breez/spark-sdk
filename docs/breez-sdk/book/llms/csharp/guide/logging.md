# Adding logging

The SDK implements detailed logging via a streaming interface you can manage within your application. The log entries are split into several levels that you can filter and store as desired within your application, for example, by appending them to a log file.

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



When several SDK instances run in the same process, for example in a [multi-user configuration](./server_mode.md), each log entry's `InstanceId` holds the identity public key of the wallet whose instance emitted it, so you can tell their logs apart. Entries not emitted by an instance leave it unset.
