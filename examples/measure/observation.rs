//! Measurement policy only: execution and semantic validation are identical.
use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};
static DETAILED: OnceLock<bool> = OnceLock::new();
pub fn configure(detailed: bool) {
    DETAILED.set(detailed).expect("measurement configured once");
}
pub fn detailed() -> bool {
    *DETAILED.get().unwrap_or(&false)
}
pub fn start(detailed: bool) -> Option<Instant> {
    detailed.then(Instant::now)
}
pub fn elapsed(start: Option<Instant>) -> Duration {
    start.map(|s| s.elapsed()).unwrap_or_default()
}
pub fn milliseconds(detailed: bool, value: Duration) -> String {
    if detailed {
        format!("{:.3}", value.as_secs_f64() * 1000.0)
    } else {
        "null".into()
    }
}
pub fn count(detailed: bool, value: u64) -> String {
    if detailed {
        value.to_string()
    } else {
        "null".into()
    }
}
