# Adding logging

The SDK implements detailed logging via a streaming interface you can manage within your application. The log entries are split into several levels that you can filter and store as desired within your application, for example, by appending them to a log file.

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



When several SDK instances run in the same process, for example in a [multi-user configuration](./server_mode.md), each log entry's `instance_id` holds the identity public key of the wallet whose instance emitted it, so you can tell their logs apart. Entries not emitted by an instance leave it unset.
