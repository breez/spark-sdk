use tracing::{Event, Subscriber};
use tracing_subscriber::{
    Layer,
    fmt::{FormatFields, format::Writer},
    layer::Context,
};
use wasm_bindgen::prelude::wasm_bindgen;

use crate::models::LogEntry;

thread_local! {
    pub(crate) static WASM_LOGGER: std::cell::RefCell<Option<Logger>> = const { std::cell::RefCell::new(None) };
}

pub struct WasmTracingLayer {}

impl<S> Layer<S> for WasmTracingLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
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
