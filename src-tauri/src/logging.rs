use std::path::Path;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, EnvFilter};

/// Holder struct for the appender's worker guard. Keeping this in managed
/// state ensures the background writer thread is flushed and joined at
/// shutdown (the guard's Drop blocks until pending log lines are written).
pub struct LogGuard(#[allow(dead_code)] WorkerGuard);

pub fn init(log_dir: &Path) -> LogGuard {
    std::fs::create_dir_all(log_dir).ok();
    let appender = tracing_appender::rolling::never(log_dir, "widget.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,ai_agent_dashboard_lib=debug"));

    fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .try_init()
        .ok();

    LogGuard(guard)
}
