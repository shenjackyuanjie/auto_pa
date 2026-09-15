//! 日志初始化：终端输出 + 按日滚动文件的组合。
//!
//! 只由 CLI 入口在分发子命令前调用一次；级别、是否写文件、文件名都由调用方决定，
//! 具体行为见 [`init`]；把错误写进日志时用 [`error_chain`] 展开全部原因。

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::prelude::*;

/// 按 `anyhow::Error` 的 `Debug` 风格展开整条错误链。
///
/// `anyhow::Error` 的 `Display`（也就是 tracing 字段里的 `%error`）只打印最外层上下文，
/// 底层原因会被丢掉：例如 `启动 AppGallery 失败` 背后的 `HDC 命令失败（退出码 …）：…`
/// 就完全看不见。`anyhow::Error` 自己用 `?error` 就能打印整条链，但驱动的 `DriverError`
/// 是 thiserror 生成的类型，`{:?}` 会变成 `HdcCommand { code: …, message: … }` 这种结构体
/// 形式。这个函数让两类错误都输出同一套排版：
///
/// ```text
/// 启动 AppGallery 失败
///
/// Caused by:
///     HDC 命令失败（退出码 Some(0)）：stdout: error: failed to start ability.
///     Error Code:10106102  Error Message:The device screen is locked …
/// ```
///
/// 参数是 `&dyn std::error::Error`，因此驱动错误可以直接传 `&error`；`anyhow::Error`
/// 没有实现 `std::error::Error`，要传 `error.as_ref()`。原因里原有的换行会保留并按层缩进。
pub fn error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut text = error.to_string();
    let causes: Vec<String> = std::iter::successors(error.source(), |cause| cause.source())
        .map(ToString::to_string)
        .collect();
    if causes.is_empty() {
        return text;
    }

    text.push_str("\n\nCaused by:");
    // 只有一个原因时 anyhow 不编号，多个原因时按 0、1、2 编号并对齐续行。
    let numbered = causes.len() > 1;
    for (index, cause) in causes.iter().enumerate() {
        let prefix = if numbered {
            format!("{index}: ")
        } else {
            String::new()
        };
        let indent = " ".repeat(4 + prefix.len());
        let mut lines = cause.lines();
        text.push_str("\n    ");
        text.push_str(&prefix);
        text.push_str(lines.next().unwrap_or_default());
        for line in lines {
            text.push('\n');
            if !line.is_empty() {
                text.push_str(&indent);
                text.push_str(line);
            }
        }
    }
    text
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    /// 单层原因：和 `anyhow` 自己的 `Debug` 排版一致。
    #[test]
    fn 错误链展开到底层原因() {
        let error = Err::<(), _>(std::io::Error::other("HDC 未返回错误文本"))
            .context("HDC 命令失败（退出码 Some(1)）")
            .unwrap_err();

        assert_eq!(
            error_chain(error.as_ref()),
            "HDC 命令失败（退出码 Some(1)）\n\nCaused by:\n    HDC 未返回错误文本"
        );
    }

    /// 多层原因时 anyhow 会给原因编号，这里必须跟着编号并对齐续行。
    #[test]
    fn 与_anyhow_的_debug_输出逐字节一致() {
        let error = Err::<(), _>(std::io::Error::other("HDC 未返回错误文本"))
            .context("HDC 命令失败（退出码 Some(1)）")
            .context("启动 AppGallery 失败")
            .unwrap_err();

        assert_eq!(error_chain(error.as_ref()), format!("{error:?}"));
    }

    #[test]
    fn 没有原因时只保留自身消息() {
        let error = anyhow::anyhow!("未发现在线 HarmonyOS 设备");
        assert_eq!(error_chain(error.as_ref()), format!("{error:?}"));
        assert_eq!(error_chain(error.as_ref()), "未发现在线 HarmonyOS 设备");
    }

    /// HDC 的多行输出要原样保留，续行按层缩进。
    #[test]
    fn 多行原因保留换行并缩进() {
        let error =
            anyhow::anyhow!("error: failed to start ability.\nError Code:10106102\n  Try again")
                .context("启动 AppGallery 失败");

        assert_eq!(error_chain(error.as_ref()), format!("{error:?}"));
        assert_eq!(
            error_chain(error.as_ref()),
            "启动 AppGallery 失败\n\nCaused by:\n    error: failed to start ability.\n    Error Code:10106102\n      Try again"
        );
    }
}
