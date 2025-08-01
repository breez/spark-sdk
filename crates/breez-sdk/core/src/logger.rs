use std::fs::OpenOptions;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::{
    EnvFilter, Layer,
    fmt::{FormatFields, format::Writer},
    layer::{Context, SubscriberExt},
    util::SubscriberInitExt,
};

use crate::{LogEntry, Logger, SdkError};

pub(crate) struct GlobalSdkLogger {
    /// Optional external log listener, that can receive a stream of log statements
    pub(crate) log_listener: Option<Box<dyn Logger>>,
}

impl<S> Layer<S> for GlobalSdkLogger
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().level() <= &Level::INFO {
            if let Some(s) = self.log_listener.as_ref() {
                let mut buf = String::new();
                let writer = Writer::new(&mut buf);

                if tracing_subscriber::fmt::format::DefaultFields::new()
                    .format_fields(writer, event)
                    .is_ok()
                {
                    s.log(LogEntry {
                        line: buf,
                        level: event.metadata().level().to_string(),
                    });
                }
            }
        }
    }
}

pub(super) fn init_logging(
    log_dir: &str,
    app_logger: Option<Box<dyn Logger>>,
    log_filter: Option<String>,
) -> Result<(), SdkError> {
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(format!("{log_dir}/sdk.log"))
        .map_err(|e| SdkError::GenericError(e.to_string()))?;

    let filter = log_filter.unwrap_or(
        "debug,h2=warn,rustls=warn,rustyline=warn,hyper=warn,hyper_util=warn,tower=warn,Connection=warn,tonic=warn".to_string(),
    );
    let subscriber = tracing_subscriber::registry()
        .with(EnvFilter::new(filter))
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_line_number(true)
                .with_writer(log_file),
        )
        .with(GlobalSdkLogger {
            log_listener: app_logger,
        });

    subscriber.init();

    Ok(())
}
