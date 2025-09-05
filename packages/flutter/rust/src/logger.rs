use crate::frb_generated::StreamSink;
pub use breez_sdk_spark::LogEntry;
use breez_sdk_spark::Logger;
use flutter_rust_bridge::frb;

#[frb(mirror(LogEntry))]
pub struct _LogEntry {
    pub line: String,
    pub level: String,
}

pub struct BindingLogger {
    pub logger: StreamSink<LogEntry>,
}

impl Logger for BindingLogger {
    fn log(&self, l: LogEntry) {
        let _ = self.logger.add(l);
    }
}
