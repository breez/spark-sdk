//! `init_logging` installs a process-wide subscriber, so its tests run in a
//! binary of their own, as one test.
#![cfg(not(all(target_family = "wasm", target_os = "unknown")))]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use breez_sdk_spark::{LogEntry, Logger, init_logging};

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<String>>>);

impl Recorder {
    fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl Logger for Recorder {
    fn log(&self, l: LogEntry) {
        self.0.lock().unwrap().push(l.line);
    }
}

fn log_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("brz-init-logging-{}-{name}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn file_contents(dir: &Path) -> String {
    fs::read_to_string(dir.join("sdk.log")).unwrap_or_default()
}

#[test]
fn a_later_call_replaces_the_logger_filter_and_log_file() {
    let (dir1, dir2) = (log_dir("1"), log_dir("2"));
    let (first, second) = (Recorder::default(), Recorder::default());

    // A file and no logger, so the `log` bridge starts at `info`.
    init_logging(
        Some(dir1.to_string_lossy().into()),
        None,
        Some("brz_init_logging=info".into()),
    )
    .unwrap();
    eprintln!(
        "[probe] after call 1: tracing max {:?}, log max {:?}",
        tracing::level_filters::LevelFilter::current(),
        log::max_level()
    );
    tracing::info!(target: "brz_init_logging", "file one");
    log::debug!(target: "brz_init_logging", "below the first filter");

    init_logging(
        Some(dir1.to_string_lossy().into()),
        Some(Box::new(first.clone())),
        Some("brz_init_logging=info".into()),
    )
    .unwrap();
    tracing::info!(target: "brz_init_logging", "logger one");
    tracing::debug!(target: "brz_init_logging", "below the filter");

    init_logging(
        Some(dir2.to_string_lossy().into()),
        Some(Box::new(second.clone())),
        Some("brz_init_logging=debug".into()),
    )
    .unwrap();
    tracing::debug!(target: "brz_init_logging", "logger two");
    // A `log` record at a level the first call's filter left out: reloading a
    // filter also updates the `log` bridge's level.
    log::debug!(target: "brz_init_logging", "through the log bridge");

    init_logging(None, None, None).unwrap();
    tracing::error!(target: "brz_init_logging", "after logging is off");

    assert_eq!(first.lines(), ["logger one"]);
    assert_eq!(second.lines(), ["logger two", "through the log bridge"]);
    let (file1, file2) = (file_contents(&dir1), file_contents(&dir2));
    assert!(
        file1.contains("file one") && file1.contains("logger one"),
        "{file1}"
    );
    assert!(!file1.contains("below the"), "{file1}");
    assert!(!file1.contains("logger two"), "{file1}");
    assert!(
        file2.contains("logger two") && file2.contains("through the log bridge"),
        "{file2}"
    );
    assert!(!file2.contains("after logging is off"), "{file2}");

    let _ = fs::remove_dir_all(dir1);
    let _ = fs::remove_dir_all(dir2);
}
