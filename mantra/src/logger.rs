use std::sync::atomic::{AtomicBool, Ordering};

/// Set to `true` if any warning was intercepted while `--warnings-as-errors` is active.
pub static HAD_WARNINGS: AtomicBool = AtomicBool::new(false);

struct WarningsAsErrorsLogger {
    inner: Box<dyn log::Log>,
    warnings_as_errors: bool,
}

impl log::Log for WarningsAsErrorsLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &log::Record) {
        if self.warnings_as_errors && record.level() == log::Level::Warn {
            HAD_WARNINGS.store(true, Ordering::Relaxed);
            let error_record = log::Record::builder()
                .level(log::Level::Error)
                .args(*record.args())
                .target(record.target())
                .module_path(record.module_path())
                .file(record.file())
                .line(record.line())
                .build();
            self.inner.log(&error_record);
        } else {
            self.inner.log(record);
        }
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

/// Initialize the global logger.
///
/// When `warnings_as_errors` is `true`, any `log::warn!` emission is re-emitted at
/// `Error` level and [`HAD_WARNINGS`] is set so the caller can exit with a non-zero
/// exit code after the command completes.
pub fn init(warnings_as_errors: bool) {
    let inner = env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .format_target(false)
        .build();

    let logger = Box::leak(Box::new(WarningsAsErrorsLogger {
        inner: Box::new(inner),
        warnings_as_errors,
    }));

    log::set_logger(logger).expect("logger must only be initialized once");
    log::set_max_level(log::LevelFilter::Info);
}
