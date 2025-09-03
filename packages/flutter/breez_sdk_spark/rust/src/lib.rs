mod frb_generated;

use extend::ext;

use breez_sdk_spark::{
    BreezSdk, EventListener, LogEntry, Logger, SdkError, SdkEvent, init_logging,
};
use frb_generated::StreamSink;

pub struct BindingEventListener {
    pub listener: StreamSink<SdkEvent>,
}

impl EventListener for BindingEventListener {
    fn on_event(&self, e: SdkEvent) {
        let _ = self.listener.add(e);
    }
}

pub struct BindingLogger {
    pub logger: StreamSink<LogEntry>,
}

impl Logger for BindingLogger {
    fn log(&self, l: LogEntry) {
        let _ = self.logger.add(l);
    }
}

fn frb_override_init_logging(
    log_dir: Option<String>,
    app_logger: Option<StreamSink<LogEntry>>,
    log_filter: Option<String>,
) -> Result<(), SdkError> {
    init_logging(
        log_dir,
        app_logger.map(|logger| Box::new(BindingLogger { logger }) as Box<dyn Logger>),
        log_filter,
    )
}

#[ext]
pub impl BreezSdk {
    fn frb_override_add_event_listener(&self, listener: StreamSink<SdkEvent>) -> String {
        self.add_event_listener(Box::new(BindingEventListener { listener }))
    }
}