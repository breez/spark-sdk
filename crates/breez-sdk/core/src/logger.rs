use std::fs::OpenOptions;
use tracing::{Event, Subscriber};
use tracing_subscriber::{
    Layer,
    filter::{LevelFilter, Targets},
    fmt::{FormatFields, format::Writer},
    layer::{Context, SubscriberExt},
    util::SubscriberInitExt,
};

use crate::{LogEntry, Logger, SdkError};

/// Default tracing filter: `info` globally, `debug` for first-party crates,
/// and noisy third-party crates silenced below `warn`. Shared with the WASM
/// bindings so both default to the same behaviour.
pub const DEFAULT_FILTER: &str = concat!(
    "info",
    // First-party crates: keep debug logging.
    ",breez_sdk_spark=debug",
    ",breez_sdk_common=debug",
    ",breez_sdk_spark_wasm=debug",
    ",breez_sdk_spark_bindings=debug",
    ",spark=debug",
    ",spark_wallet=debug",
    ",spark_postgres=debug",
    ",spark_mysql=debug",
    ",flashnet=debug",
    ",platform_utils=debug",
    // Noisy third-party crates: silence below warn.
    ",h2=warn",
    ",rustls=warn",
    ",rustyline=warn",
    ",hyper=warn",
    ",hyper_util=warn",
    ",tower=warn",
    ",Connection=warn",
    ",tonic=warn",
);

/// Parses a log filter in `RUST_LOG` syntax: comma-separated `target=level`
/// directives and bare levels, where the most specific target wins. Directives
/// that filter on spans or fields (`target[span{field=value}]=level`) are not
/// supported and are ignored, like any other directive that does not parse.
#[doc(hidden)]
pub fn parse_log_filter(filter: &str) -> Targets {
    let directives: Vec<_> = filter
        .split(',')
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .filter_map(parse_directive)
        .collect();
    // Like `EnvFilter`, a filter with no usable directive shows errors only.
    if directives.is_empty() {
        return Targets::new().with_default(LevelFilter::ERROR);
    }
    directives
        .into_iter()
        .fold(Targets::new(), |targets, (target, level)| match target {
            Some(target) => targets.with_target(target, level),
            None => targets.with_default(level),
        })
}

/// A target (`None` for a bare level) and its level. A target without a level,
/// or with an empty one, enables every level.
fn parse_directive(directive: &str) -> Option<(Option<&str>, LevelFilter)> {
    let first = directive.chars().next()?;
    let valid_start = first.is_alphanumeric() || matches!(first, '-' | ':' | '_');
    if !valid_start || directive.contains(['[', '{']) {
        return None;
    }
    match directive.split_once('=') {
        Some((target, level)) => {
            let level = match level.trim() {
                "" => LevelFilter::TRACE,
                level => level.parse().ok()?,
            };
            Some((Some(target.trim()), level))
        }
        None => Some(match directive.parse() {
            Ok(level) => (None, level),
            Err(_) => (Some(directive), LevelFilter::TRACE),
        }),
    }
}

pub(crate) struct GlobalSdkLogger {
    /// Optional external log listener, that can receive a stream of log statements
    pub(crate) log_listener: Option<Box<dyn Logger>>,
}

impl<S> Layer<S> for GlobalSdkLogger
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
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

pub(super) fn init_logging(
    log_dir: Option<&str>,
    app_logger: Option<Box<dyn Logger>>,
    log_filter: Option<&str>,
) -> Result<(), SdkError> {
    let filter = log_filter.unwrap_or(DEFAULT_FILTER);

    let registry = tracing_subscriber::registry().with(
        GlobalSdkLogger {
            log_listener: app_logger,
        }
        .with_filter(parse_log_filter(filter)),
    );

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
        let fmt_layer = fmt_layer.with_filter(parse_log_filter(filter));
        registry.with(fmt_layer).try_init()?;
    } else {
        registry.try_init()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracing::{debug, error, info, trace, warn};
    use tracing_subscriber::{EnvFilter, Registry, layer::Filter};

    use super::{
        Context, DEFAULT_FILTER, Event, GlobalSdkLogger, Layer, SubscriberExt, parse_log_filter,
    };
    use crate::{LogEntry, Logger};

    /// External logger that records the level of every entry it receives.
    struct CapturingLogger {
        levels: Arc<Mutex<Vec<String>>>,
    }

    impl Logger for CapturingLogger {
        fn log(&self, l: LogEntry) {
            self.levels.lock().unwrap().push(l.level);
        }
    }

    /// Runs `emit` with a [`GlobalSdkLogger`] filtered by `filter` installed as
    /// the thread-local default, returning the levels that reached the external
    /// logger. `emit` must use a literal `target:` (the macros bake it into a
    /// `static` callsite) matching a directive in `filter`.
    fn forwarded_levels(filter: &str, emit: impl FnOnce()) -> Vec<String> {
        let levels = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(
            GlobalSdkLogger {
                log_listener: Some(Box::new(CapturingLogger {
                    levels: levels.clone(),
                })),
            }
            .with_filter(parse_log_filter(filter)),
        );

        tracing::subscriber::with_default(subscriber, emit);

        levels.lock().unwrap().clone()
    }

    #[test]
    fn external_logger_respects_filter_level() {
        // A `debug` filter forwards debug (and above) but not trace — this is
        // the behaviour that regressed when the layer hard-capped at INFO.
        let levels = forwarded_levels("brz_logtest_debug=debug", || {
            info!(target: "brz_logtest_debug", "info");
            debug!(target: "brz_logtest_debug", "debug");
            trace!(target: "brz_logtest_debug", "trace");
        });
        assert!(levels.contains(&"INFO".to_string()), "got {levels:?}");
        assert!(levels.contains(&"DEBUG".to_string()), "got {levels:?}");
        assert!(!levels.contains(&"TRACE".to_string()), "got {levels:?}");

        // A `trace` filter forwards trace too.
        let levels = forwarded_levels("brz_logtest_trace=trace", || {
            debug!(target: "brz_logtest_trace", "debug");
            trace!(target: "brz_logtest_trace", "trace");
        });
        assert!(levels.contains(&"DEBUG".to_string()), "got {levels:?}");
        assert!(levels.contains(&"TRACE".to_string()), "got {levels:?}");
    }

    /// Records the target and level of every event that passes the filter.
    struct EventRecorder(Arc<Mutex<Vec<String>>>);

    impl<S: tracing::Subscriber> Layer<S> for EventRecorder {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let meta = event.metadata();
            self.0
                .lock()
                .unwrap()
                .push(format!("{}:{}", meta.target(), meta.level()));
        }
    }

    /// Emits an event at every level for a fixed set of targets and returns
    /// the ones `filter` lets through.
    fn enabled_events(filter: impl Filter<Registry> + Send + Sync + 'static) -> Vec<String> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber =
            tracing_subscriber::registry().with(EventRecorder(events.clone()).with_filter(filter));
        tracing::subscriber::with_default(subscriber, || {
            macro_rules! at_all_levels {
                ($($target:literal),*) => {$(
                    trace!(target: $target, "");
                    debug!(target: $target, "");
                    info!(target: $target, "");
                    warn!(target: $target, "");
                    error!(target: $target, "");
                )*};
            }
            at_all_levels!(
                "breez_sdk_spark",
                "breez_sdk_spark::sdk",
                "spark",
                "spark::tree",
                "sparkle",
                "h2::codec",
                "Connection",
                "other"
            );
        });
        events.lock().unwrap().clone()
    }

    #[test]
    fn log_filter_matches_env_filter() {
        let filters = [
            DEFAULT_FILTER,
            "",
            "info",
            "debug",
            "TRACE",
            "5",
            "off",
            "warn,spark=trace",
            "spark",
            "spark=",
            "spark::=debug",
            "spark=debug,spark=warn",
            "debug,info",
            "info,debug",
            "info,,debug",
            "spark=debug,",
            "debug,spark=off",
            "error,breez_sdk_spark::sdk=trace,breez_sdk_spark=info",
            "garbage!!,debug",
            "=debug",
            "spark=verbose",
            "info;spark=debug",
            "Spark=debug",
        ];
        for filter in filters {
            assert_eq!(
                enabled_events(parse_log_filter(filter)),
                enabled_events(EnvFilter::new(filter)),
                "filter `{filter}`"
            );
        }
    }

    #[test]
    fn log_filter_ignores_span_and_field_directives() {
        assert_eq!(
            enabled_events(parse_log_filter("warn,spark[sync]=debug,[{id=1}]=trace")),
            enabled_events(parse_log_filter("warn"))
        );
    }

    #[test]
    fn log_filter_tolerates_whitespace() {
        assert_eq!(
            enabled_events(parse_log_filter("info, spark = debug ")),
            enabled_events(parse_log_filter("info,spark=debug"))
        );
    }
}
