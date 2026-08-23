use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::prelude::*;

/// 初始化终端与按日滚动的日志文件输出。
pub fn init(verbose: bool, disable_log_file: bool, file_name: &str) -> Option<WorkerGuard> {
    let level = if verbose {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    if disable_log_file {
        tracing_subscriber::fmt()
            .with_max_level(level)
            .with_target(false)
            .init();
        return None;
    }

    let appender = tracing_appender::rolling::daily("logs", file_name);
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_ansi(true)
        .with_target(false)
        .with_writer(std::io::stdout);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(false)
        .with_writer(writer);

    tracing_subscriber::registry()
        .with(tracing_subscriber::filter::LevelFilter::from_level(level))
        .with(stdout_layer)
        .with(file_layer)
        .init();
    Some(guard)
}
