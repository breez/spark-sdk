# Adding logging

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/fn.init_logging.html

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
