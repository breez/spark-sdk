# Adding logging

API docs: https://breez.github.io/spark-sdk/breez_sdk_spark/fn.init_logging.html

The SDK implements detailed logging via a streaming interface you can manage within your application. The log entries are split into several levels that you can filter and store as desired within your application, for example, by appending them to a log file.

```rust
let data_dir_path = PathBuf::from(&data_dir);
fs::create_dir_all(data_dir_path)?;

init_logging(Some(data_dir), None, None)?;
```
