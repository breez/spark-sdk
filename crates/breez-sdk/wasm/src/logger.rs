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
use wasm_bindgen::prelude::wasm_bindgen;

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
