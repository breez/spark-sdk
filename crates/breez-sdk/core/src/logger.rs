use std::fs::OpenOptions;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::{
    EnvFilter, Layer,
    fmt::{FormatFields, format::Writer},
    layer::{Context, SubscriberExt},
    util::SubscriberInitExt,
};

use crate::{LogEntry, Logger, SdkError};

/// Tracing target reserved for the bench harness's span instrumentation
/// (operator-RPC + SSP-RPC `#[tracing::instrument]` attributes). The
/// per-target `=off` directive appended below silences these spans on
/// every layer except the dedicated bench layer — so an SDK integrator
/// running at `debug` does NOT get close-event noise in their log file
/// or their `app_logger` callback. The bench layer is added only when
/// the caller's filter explicitly re-enables this target at a non-`off`
/// level (i.e. the bench server's `--bench-trace` preset).
const BENCH_SPAN_TARGET: &str = "breez_sdk_core::send_phases";

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

/// Returns true if `filter` contains a directive enabling the bench
/// span target at any non-`off` level. Used to decide whether to add
/// the dedicated bench-span layer in `init_logging`.
fn bench_span_layer_requested(filter: &str) -> bool {
    for part in filter.split(',') {
        let Some((target, level)) = part.split_once('=') else {
            continue;
        };
        if target.trim() == BENCH_SPAN_TARGET && !level.trim().eq_ignore_ascii_case("off") {
            return true;
        }
    }
    false
}

/// Builds the filter applied to every "safe" layer (app-logger callback +
/// main file layer). Always forces `BENCH_SPAN_TARGET=off` regardless of
/// what the caller supplied — `EnvFilter`'s specificity resolution makes
/// the per-target directive override any global level, so an integrator
/// at `debug` cannot accidentally pick up bench-instrumentation spans.
fn safe_filter(user_filter: &str) -> EnvFilter {
    EnvFilter::new(format!("{user_filter},{BENCH_SPAN_TARGET}=off"))
}

pub(super) fn init_logging(
    log_dir: Option<&str>,
    app_logger: Option<Box<dyn Logger>>,
    log_filter: Option<&str>,
) -> Result<(), SdkError> {
    let user_filter = log_filter.unwrap_or(DEFAULT_FILTER);
    let bench_layer_enabled = bench_span_layer_requested(user_filter);

    // Build the layer stack. Every layer carries its own filter
    // (per-layer filters are the canonical pattern in tracing-subscriber
    // when different layers need different visibility) so the bench
    // target is provably silenced on the app-logger callback and main
    // file writer while still being available to the dedicated bench
    // layer when explicitly requested.
    let app_logger_layer = GlobalSdkLogger {
        log_listener: app_logger,
    }
    .with_filter(safe_filter(user_filter));

    let main_fmt_layer = if let Some(dir) = log_dir {
        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(format!("{dir}/sdk.log"))
            .map_err(|e| SdkError::Generic(e.to_string()))?;
        Some(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_line_number(true)
                .with_writer(log_file)
                .with_filter(safe_filter(user_filter)),
        )
    } else {
        None
    };

    // Dedicated bench layer: only the bench span target, with span
    // CLOSE events synthesized. The fmt layer's CLOSE event carries
    // `time.busy` / `time.idle`, giving us per-RPC elapsed time for
    // free — every `#[tracing::instrument]`-decorated SSP / operator
    // method emits one line at span close, with the full parent-span
    // hierarchy (e.g. `send_payment{payment_id=...}:request_lightning_send`).
    let bench_layer = if bench_layer_enabled {
        let dir = log_dir.ok_or_else(|| {
            SdkError::Generic(
                "bench-span tracing was requested but no log_dir was provided to init_logging"
                    .to_string(),
            )
        })?;
        let bench_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(format!("{dir}/sdk.log"))
            .map_err(|e| SdkError::Generic(e.to_string()))?;
        // Restrict to the bench target. The `off` floor for everything
        // else keeps unrelated spans (e.g. swap-detection `trace`
        // events under spark::tree::service) from being CLOSE-rendered
        // on this layer — they still hit the main layer as regular log
        // lines.
        let filter = EnvFilter::new(format!("off,{BENCH_SPAN_TARGET}=info"));
        Some(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_line_number(false)
                .with_writer(bench_file)
                .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                .with_filter(filter),
        )
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(app_logger_layer)
        .with(main_fmt_layer)
        .with(bench_layer)
        .try_init()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bench_layer_detected_only_when_target_enabled() {
        // Default filter: no mention of the bench target.
        assert!(!bench_span_layer_requested(DEFAULT_FILTER));
        // Plain global levels — should not light up the bench layer.
        assert!(!bench_span_layer_requested("debug"));
        assert!(!bench_span_layer_requested("trace"));
        // Caller silenced the bench target explicitly — still false.
        assert!(!bench_span_layer_requested(
            "debug,breez_sdk_core::send_phases=off",
        ));
        // Bench preset — true.
        assert!(bench_span_layer_requested(
            "spark::tree::service=trace,breez_sdk_core::send_phases=info,error",
        ));
        // Case-insensitive on the `off` check.
        assert!(!bench_span_layer_requested(
            "debug,breez_sdk_core::send_phases=OFF",
        ));
    }

    /// Prod-safety: at `debug` (the recommended integrator level), an
    /// info-level span on the bench target must be silenced on the
    /// `app_logger` forwarder. This is the layer that bridges Rust
    /// tracing into integrator log callbacks, so it's the highest-risk
    /// surface for accidental leakage.
    /// Capture-only logger used by the debug-integrator prod-safety
    /// test below. Records every `LogEntry` so the test can assert
    /// which targets reached the integrator-side callback.
    #[derive(Default)]
    struct CaptureLogger {
        entries: std::sync::Mutex<Vec<LogEntry>>,
    }

    impl Logger for CaptureLogger {
        fn log(&self, l: LogEntry) {
            self.entries.lock().unwrap().push(l);
        }
    }

    /// Forwarding shim so the boxed `dyn Logger` and the `Arc<CaptureLogger>`
    /// in the test share the same capture state.
    struct ForwardingLogger(std::sync::Arc<CaptureLogger>);

    impl Logger for ForwardingLogger {
        fn log(&self, l: LogEntry) {
            self.0.log(l);
        }
    }

    /// Confirms the close-event format produced when the bench layer
    /// is active. This is the format `aggregate.py` will parse, so we
    /// pin it here — any future tracing-subscriber upgrade that
    /// changes the line layout would break the bench report and
    /// surface here first.
    #[test]
    fn bench_layer_emits_close_events_with_span_hierarchy() {
        use std::io::Write;
        use std::sync::Mutex;
        use tracing_subscriber::fmt::format::FmtSpan;
        use tracing_subscriber::layer::SubscriberExt;

        // In-memory writer so we can assert on the produced output
        // without touching the filesystem.
        #[derive(Clone, Default)]
        struct BufWriter(std::sync::Arc<Mutex<Vec<u8>>>);

        impl Write for BufWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufWriter {
            type Writer = BufWriter;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let buf = BufWriter::default();
        let buf_for_assert = buf.0.clone();

        // Same layer config init_logging builds in bench mode, minus
        // the file open + global init.
        let filter = EnvFilter::new(format!("off,{BENCH_SPAN_TARGET}=info"));
        let layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(buf)
            .with_span_events(FmtSpan::CLOSE)
            .with_filter(filter);

        let sub = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(sub, || {
            // Outer span (mirrors what a top-level `send_payment`
            // instrument would produce) with a recorded field.
            let outer = tracing::info_span!(
                target: "breez_sdk_core::send_phases",
                "send_payment",
                payment_id = "pid-abc",
            );
            let _o = outer.enter();
            // Inner span (mirrors an SSP / operator RPC). On close it
            // should render the parent hierarchy so aggregate.py can
            // join the RPC timing back to the payment.
            let inner = tracing::info_span!(
                target: "breez_sdk_core::send_phases",
                "request_lightning_send",
                operator_id = "so-3",
            );
            let _i = inner.enter();
            std::thread::sleep(std::time::Duration::from_millis(2));
        });

        let out = String::from_utf8(buf_for_assert.lock().unwrap().clone()).unwrap();
        // We expect at least two close lines (inner first, then outer).
        let close_lines: Vec<&str> = out.lines().filter(|l| l.contains("close")).collect();
        assert!(
            close_lines.len() >= 2,
            "expected ≥2 close events, got: {out}",
        );
        // The inner span's close line must carry the parent payment_id
        // (via the span hierarchy) and the inner operator_id.
        let inner_close = close_lines
            .iter()
            .find(|l| l.contains("request_lightning_send"))
            .unwrap_or_else(|| panic!("no inner close line in: {out}"));
        assert!(
            inner_close.contains("payment_id"),
            "inner close missing parent payment_id: {inner_close}",
        );
        assert!(
            inner_close.contains("operator_id"),
            "inner close missing operator_id: {inner_close}",
        );
        // `time.busy` is the elapsed entered-time; presence is the
        // signal `aggregate.py` will key on.
        assert!(
            inner_close.contains("time.busy"),
            "inner close missing time.busy: {inner_close}",
        );
    }

    #[test]
    fn debug_level_integrator_does_not_see_bench_spans() {
        use tracing_subscriber::layer::SubscriberExt;

        let captured = std::sync::Arc::new(CaptureLogger::default());

        // Mirror what an integrator at `debug` would set up. Build the
        // same layer stack `init_logging` constructs, but don't call
        // `try_init` (would conflict with other tests in the same
        // process). Test the layer behaviour directly.
        let user_filter = DEFAULT_FILTER;
        let app_logger: Box<dyn Logger> = Box::new(ForwardingLogger(captured.clone()));
        let layer = GlobalSdkLogger {
            log_listener: Some(app_logger),
        }
        .with_filter(safe_filter(user_filter));

        let sub = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(sub, || {
            // Regular info event on a non-bench target — should pass.
            tracing::info!(target: "breez_sdk_core", "regular log");
            // Info event on the bench target — must be silenced.
            tracing::info!(
                target: "breez_sdk_core::send_phases",
                payment_id = "p1",
                "should not appear",
            );
            // A bench-target span entering/closing — also silenced.
            let s = tracing::info_span!(
                target: "breez_sdk_core::send_phases",
                "bench_span",
                payment_id = "p2",
            );
            let _g = s.enter();
        });

        let entries = captured.entries.lock().unwrap();
        let lines: Vec<&str> = entries.iter().map(|e| e.line.as_str()).collect();
        assert!(
            lines.iter().any(|l| l.contains("regular log")),
            "regular log was unexpectedly filtered out: {lines:?}",
        );
        assert!(
            lines.iter().all(|l| !l.contains("should not appear")
                && !l.contains("bench_span")
                && !l.contains("p1")
                && !l.contains("p2")),
            "bench-target events leaked into app logger: {lines:?}",
        );
    }
}
