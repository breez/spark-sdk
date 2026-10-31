use tracing::{
    Event, Subscriber,
    span::{Attributes, Id},
};
use tracing_subscriber::{
    Layer,
    fmt::{FormatFields, format::Writer},
    layer::Context,
    registry::LookupSpan,
};
use wasm_bindgen::{JsValue, prelude::wasm_bindgen};
use web_sys::console;

use crate::models::LogEntry;

thread_local! {
    pub(crate) static WASM_LOGGER: std::cell::RefCell<Option<Logger>> = const { std::cell::RefCell::new(None) };
}

pub struct WasmTracingLayer {}

impl<S> Layer<S> for WasmTracingLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        breez_sdk_spark::record_instance_id(attrs, id, &ctx);
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        WASM_LOGGER.with_borrow(|logger| {
            if let Some(logger) = logger.as_ref() {
                let mut buf = String::new();
                let writer = Writer::new(&mut buf);

                if tracing_subscriber::fmt::format::DefaultFields::new()
                    .format_fields(writer, event)
                    .is_ok()
                {
                    logger.log(LogEntry {
                        line: buf,
                        level: event.metadata().level().to_string(),
                        instance_id: breez_sdk_spark::event_instance_id(event, &ctx),
                    });
                }
            }
        });
    }
}

#[wasm_bindgen(typescript_custom_section)]
const LOGGER: &'static str = r#"export interface Logger {
    log: (l: LogEntry) => void;
}"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "Logger")]
    pub type Logger;

    #[wasm_bindgen(structural, method, js_name = log)]
    pub fn log(this: &Logger, l: LogEntry);
}

/// A logger that prints each log entry to the JavaScript console, with the console method that
/// matches the entry's level.
#[wasm_bindgen]
#[derive(Default)]
pub struct ConsoleLogger;

#[wasm_bindgen]
impl ConsoleLogger {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self
    }

    pub fn log(&self, entry: LogEntry) {
        let LogEntry { line, level, .. } = entry;
        let level = level.to_ascii_uppercase();
        let text = JsValue::from(format!("[{level}] {line}"));
        match level.as_str() {
            "ERROR" => console::error_1(&text),
            "WARN" => console::warn_1(&text),
            "INFO" => console::info_1(&text),
            _ => console::debug_1(&text),
        }
    }
}
