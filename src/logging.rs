//! 日志初始化：终端输出 + 按日滚动文件的组合。
//!
//! 只由 CLI 入口在分发子命令前调用一次；级别、是否写文件、文件名都由调用方决定，
//! 具体行为见 [`init`]。

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::prelude::*;

/// 初始化终端与按日滚动的日志文件输出。
///
/// 级别默认是 `INFO`，`verbose` 为真时提升到 `DEBUG`，终端与文件共用同一级别；
/// 终端保留 ANSI 颜色，文件不写颜色转义。
///
/// 文件写在 `logs/` 目录下、按日滚动，实际文件名是 `<file_name>.<日期>`，
/// 例如 `logs/search-rust.log.2026-09-12`；`disable_log_file` 为真时只初始化终端输出
/// 并返回 `None`。
///
/// 文件写入是非阻塞的：日志先投递到后台线程，由返回的 [`WorkerGuard`] 在析构时负责
/// 冲刷并结束该线程。调用方必须让这个守卫活到流程结束，否则尾部日志可能丢失。
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
