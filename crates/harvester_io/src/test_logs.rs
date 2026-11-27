use std::cell::RefCell;
use std::sync::Once;

thread_local! {
    static MESSAGES: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

struct TestLogger;

impl log::Log for TestLogger {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        MESSAGES.with(|messages| messages.borrow().is_some())
    }

    fn log(&self, record: &log::Record<'_>) {
        MESSAGES.with(|messages| {
            if let Some(messages) = messages.borrow_mut().as_mut() {
                messages.push(record.args().to_string());
            }
        });
    }

    fn flush(&self) {}
}

pub(crate) fn initialize() {
    static LOGGER: TestLogger = TestLogger;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        log::set_logger(&LOGGER).unwrap();
        log::set_max_level(log::LevelFilter::Info);
    });
}

pub(crate) fn capture<T>(run: impl FnOnce() -> T) -> (T, Vec<String>) {
    initialize();
    MESSAGES.with(|messages| assert!(messages.replace(Some(Vec::new())).is_none()));
    let result = run();
    let messages = MESSAGES.with(|messages| messages.take().unwrap());
    (result, messages)
}
