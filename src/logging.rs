use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::writer::MakeWriterExt;

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
    let writer = std::io::stdout.and(writer);
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        .with_writer(writer)
        .init();
    Some(guard)
}
