use std::fs::OpenOptions;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::{
    EnvFilter, Layer,
    fmt::{FormatFields, format::Writer},
    layer::{Context, SubscriberExt},
    util::SubscriberInitExt,
};

use crate::{LogEntry, Logger, SdkError};

const DEFAULT_FILTER: &str = "debug,h2=warn,rustls=warn,rustyline=warn,hyper=warn,hyper_util=warn,\
     tower=warn,Connection=warn,tonic=warn";

pub(crate) struct GlobalSdkLogger {
    /// Optional external log listener, that can receive a stream of log statements
    pub(crate) log_listener: Option<Box<dyn Logger>>,
}

impl<S> Layer<S> for GlobalSdkLogger
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().level() <= &Level::INFO
            && let Some(s) = self.log_listener.as_ref()
        {
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

pub(super) fn init_logging(
    log_dir: Option<&str>,
    app_logger: Option<Box<dyn Logger>>,
    log_filter: Option<&str>,
) -> Result<(), SdkError> {
    let filter = log_filter.unwrap_or(DEFAULT_FILTER);

    let registry = tracing_subscriber::registry()
        .with(EnvFilter::new(filter))
        .with(GlobalSdkLogger {
            log_listener: app_logger,
        });

    if let Some(log_dir) = log_dir {
        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(format!("{log_dir}/sdk.log"))
            .map_err(|e| SdkError::Generic(e.to_string()))?;
        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_line_number(true)
            .with_writer(log_file);
        // Bench-only: render span CLOSE lines (`time.busy` / `time.idle`)
        // so the breez-bench aggregator can attribute per-RPC latency.
        // The user's filter (`spark::operator_rpc=info`, etc.) controls
        // which spans actually emit.
        #[cfg(feature = "span-trace")]
        let fmt_layer = fmt_layer.with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE);
        registry.with(fmt_layer).try_init()?;
    } else {
        registry.try_init()?;
    }

    Ok(())
}
