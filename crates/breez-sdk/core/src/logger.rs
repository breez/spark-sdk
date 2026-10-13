use std::{fmt::Debug, fs::OpenOptions};
use tracing::{
    Event, Span, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id},
};
use tracing_subscriber::{
    Layer,
    filter::{LevelFilter, Targets},
    fmt::{FormatFields, format::Writer},
    layer::{Context, SubscriberExt},
    registry::LookupSpan,
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

/// Target of the spans that carry an SDK instance's identifier in their
/// `instance` field. It must match the target `macros::instance_span` emits.
const INSTANCE_SPAN_TARGET: &str = "breez_sdk_spark_instance";
const INSTANCE_FIELD: &str = "instance";

/// The filter of the layers that forward log entries to the app: `filter` in
/// `RUST_LOG` syntax, or [`DEFAULT_FILTER`], with instance spans always enabled
/// so that every entry that passes it can be attributed to its instance.
#[doc(hidden)]
pub fn sdk_log_filter(filter: Option<&str>) -> Targets {
    parse_log_filter(filter.unwrap_or(DEFAULT_FILTER))
        .with_target(INSTANCE_SPAN_TARGET, LevelFilter::TRACE)
}

/// The filter of the log file. Instance spans are left out: the file prints the
/// spans each line was emitted in, and every line would carry the identity key.
fn file_log_filter(filter: Option<&str>) -> Targets {
    parse_log_filter(filter.unwrap_or(DEFAULT_FILTER))
        .with_target(INSTANCE_SPAN_TARGET, LevelFilter::OFF)
}

/// A span attributing what runs in it to the SDK instance `instance_id`.
pub(crate) fn instance_span(instance_id: &str) -> Span {
    tracing::error_span!(target: INSTANCE_SPAN_TARGET, "sdk", instance = instance_id)
}

/// Parses a log filter in `RUST_LOG` syntax: comma-separated `target=level`
/// directives and bare levels, where the most specific target wins. Directives
/// that filter on spans or fields (`target[span{field=value}]=level`) are not
/// supported and are ignored, like any other directive that does not parse.
fn parse_log_filter(filter: &str) -> Targets {
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

/// An instance identifier, stored in the extensions of the span that carries it.
struct InstanceId(String);

#[derive(Default)]
struct InstanceVisitor(Option<String>);

impl Visit for InstanceVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == INSTANCE_FIELD {
            self.0 = Some(value.to_string());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == INSTANCE_FIELD {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

/// Stores the instance identifier of a new instance span, for
/// [`event_instance_id`] to find. Call from `Layer::on_new_span`.
#[doc(hidden)]
pub fn record_instance_id<S>(attrs: &Attributes<'_>, id: &Id, ctx: &Context<'_, S>)
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    if attrs.metadata().target() != INSTANCE_SPAN_TARGET {
        return;
    }
    let mut visitor = InstanceVisitor::default();
    attrs.record(&mut visitor);
    if let (Some(instance_id), Some(span)) = (visitor.0, ctx.span(id)) {
        span.extensions_mut().replace(InstanceId(instance_id));
    }
}

/// The identifier of the SDK instance an event was emitted in: the one carried
/// by its closest instance span.
#[doc(hidden)]
pub fn event_instance_id<S>(event: &Event<'_>, ctx: &Context<'_, S>) -> Option<String>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    ctx.event_scope(event)?.find_map(|span| {
        span.extensions()
            .get::<InstanceId>()
            .map(|instance| instance.0.clone())
    })
}

pub(crate) struct GlobalSdkLogger {
    /// Optional external log listener, that can receive a stream of log statements
    pub(crate) log_listener: Option<Box<dyn Logger>>,
}

impl<S> Layer<S> for GlobalSdkLogger
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        record_instance_id(attrs, id, &ctx);
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
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
                    instance_id: event_instance_id(event, &ctx),
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
    let registry = tracing_subscriber::registry().with(
        GlobalSdkLogger {
            log_listener: app_logger,
        }
        .with_filter(sdk_log_filter(log_filter)),
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
        let fmt_layer = fmt_layer.with_filter(file_log_filter(log_filter));
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
        Context, DEFAULT_FILTER, Event, GlobalSdkLogger, Layer, SubscriberExt, file_log_filter,
        instance_span, parse_log_filter, sdk_log_filter,
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

    /// External logger that records the message and instance of every entry.
    #[derive(Clone, Default)]
    struct EntryRecorder(Arc<Mutex<Vec<LogEntry>>>);

    impl Logger for EntryRecorder {
        fn log(&self, l: LogEntry) {
            self.0.lock().unwrap().push(l);
        }
    }

    impl EntryRecorder {
        /// A subscriber forwarding to this recorder through `sdk_log_filter(filter)`.
        fn subscriber(&self, filter: &str) -> impl tracing::Subscriber + Send + Sync {
            tracing_subscriber::registry().with(
                GlobalSdkLogger {
                    log_listener: Some(Box::new(self.clone())),
                }
                .with_filter(sdk_log_filter(Some(filter))),
            )
        }

        fn instance_of(&self, line: &str) -> Option<String> {
            let entries = self.0.lock().unwrap();
            let entry = entries
                .iter()
                .find(|entry| entry.line == line)
                .unwrap_or_else(|| panic!("`{line}` not logged, got {entries:?}"));
            entry.instance_id.clone()
        }
    }

    #[test]
    fn external_logger_receives_closest_instance() {
        let recorder = EntryRecorder::default();
        tracing::subscriber::with_default(recorder.subscriber("brz_logtest_instance=info"), || {
            info!(target: "brz_logtest_instance", "outside");
            instance_span("alice").in_scope(|| {
                // Spans between the instance span and the event don't hide it.
                tracing::info_span!(target: "brz_logtest_instance", "inner").in_scope(|| {
                    info!(target: "brz_logtest_instance", "in alice");
                });
                instance_span("bob").in_scope(|| {
                    info!(target: "brz_logtest_instance", "in bob");
                });
            });
        });
        assert_eq!(recorder.instance_of("outside"), None);
        assert_eq!(recorder.instance_of("in alice").as_deref(), Some("alice"));
        assert_eq!(recorder.instance_of("in bob").as_deref(), Some("bob"));
    }

    #[test]
    fn instance_survives_filters_that_exclude_its_span() {
        let recorder = EntryRecorder::default();
        // `off` disables every target the filter doesn't name, the instance
        // span's included.
        let filter = "off,brz_logtest_filtered=warn";
        tracing::subscriber::with_default(recorder.subscriber(filter), || {
            instance_span("alice").in_scope(|| {
                warn!(target: "brz_logtest_filtered", "warned");
            });
        });
        assert_eq!(recorder.instance_of("warned").as_deref(), Some("alice"));
    }

    #[test]
    fn log_file_leaves_out_instance_spans() {
        #[derive(Clone, Default)]
        struct Buffer(Arc<Mutex<Vec<u8>>>);

        impl std::io::Write for Buffer {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let buffer = Buffer::default();
        let writer = buffer.clone();
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(move || writer.clone())
                .with_filter(file_log_filter(None)),
        );
        tracing::subscriber::with_default(subscriber, || {
            instance_span("alice").in_scope(|| {
                tracing::info_span!(target: "brz_logtest_file", "inner").in_scope(|| {
                    info!(target: "brz_logtest_file", "logged");
                });
            });
        });

        let output = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        assert!(
            output.contains("inner: brz_logtest_file: logged"),
            "got {output}"
        );
        assert!(!output.contains("alice"), "got {output}");
    }

    struct Instance {
        instance_id: String,
    }

    #[macros::instance_span]
    impl Instance {
        pub async fn log_here_and_in_task(&self) {
            use tracing::Instrument;
            info!(target: "brz_logtest_macro", "in method");
            platform_utils::tokio::spawn(
                async { info!(target: "brz_logtest_macro", "in task") }.in_current_span(),
            )
            .await
            .unwrap();
        }
    }

    #[macros::async_test_all]
    async fn instance_span_macro_attributes_method_and_spawned_tasks() {
        let recorder = EntryRecorder::default();
        let _guard =
            tracing::subscriber::set_default(recorder.subscriber("brz_logtest_macro=info"));
        Instance {
            instance_id: "alice".to_string(),
        }
        .log_here_and_in_task()
        .await;
        assert_eq!(recorder.instance_of("in method").as_deref(), Some("alice"));
        assert_eq!(recorder.instance_of("in task").as_deref(), Some("alice"));
    }
}
