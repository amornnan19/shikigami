use jiff::Timestamp;

#[cfg(feature = "e2e")]
pub const CLOCK_FILE_ENV: &str = "SHIKIGAMI_E2E_CLOCK_FILE";

pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

#[cfg(feature = "e2e")]
pub struct FileClock(pub std::path::PathBuf);

#[cfg(feature = "e2e")]
impl Clock for FileClock {
    fn now(&self) -> Timestamp {
        std::fs::read_to_string(&self.0)
            .ok()
            .and_then(|text| text.trim().parse::<i64>().ok())
            .and_then(|ms| Timestamp::from_millisecond(ms).ok())
            .unwrap_or_else(Timestamp::now)
    }
}

pub fn current() -> Box<dyn Clock> {
    #[cfg(feature = "e2e")]
    if let Some(path) = std::env::var_os(CLOCK_FILE_ENV).filter(|path| !path.is_empty()) {
        return Box::new(FileClock(path.into()));
    }
    Box::new(SystemClock)
}

pub fn now() -> Timestamp {
    current().now()
}
