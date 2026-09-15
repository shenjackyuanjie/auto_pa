//! `auto-pa` 的命令行入口。
//!
//! 这里只做三件事：解析参数、按子命令初始化日志、把参数交给库侧的
//! `search::run` / `hilog::run`。设备发现、UI 遍历等流程都放在库（`auto_pa_rs`）里，
//! 以便单独测试。
//!
//! 两个子命令各写一份日志文件（`search-rust.log` / `hilog-rust.log`，按日滚动），
//! 因此日志必须在分发到 `run` 之前完成初始化。

use anyhow::Result;
use auto_pa_rs::command::{hilog, search};
use auto_pa_rs::logging;
use clap::{Parser, Subcommand};

/// `auto-pa` 的顶层参数：没有全局选项，唯一的参数是必选的子命令。
// 顶层 about 由下面的 `#[command(about = ...)]` 显式给出。这里刻意只写一段，
// 避免文档注释生成 long_about 而改变 `--help` 的现有输出。
#[derive(Debug, Parser)]
#[command(name = "auto-pa", about = "AppGallery 自动化工具")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// 支持的两个子命令，各自带独立参数、各自写一份日志文件。
// 该枚举的文档注释会被 clap 用作顶层命令的 about；与 `Cli` 同理，这里保持单段，
// 不影响 `auto-pa --help` 的现有输出（子命令自身的说明写在各自分支上）。
#[derive(Debug, Subcommand)]
enum Command {
    /// 收集 AppGallery 分类应用名称并逐个搜索。
    ///
    /// 先遍历分类收集应用名并保存进度，再逐个搜索；搜索成功后会打开第一条结果，
    /// 收集「同开发者的应用」（用 `--skip-developer` 可跳过这一步）。
    /// 多台在线设备并行执行，任何一台失败都会在全部结束后汇总上报。
    Search(search::SearchArgs),
    /// 遍历 AppGallery 分类和应用列表；默认不抓取 hilog、不提交应用。
    ///
    /// 每轮都会重新发现设备并并行遍历，`--loop` 控制轮数、`--loop-wait` 控制轮间隔。
    /// `--submit`（抓取 hilog 并投稿）尚未实现，启用它会直接报错退出。
    Hilog(hilog::HilogArgs),
}

/// 解析命令行并分发到对应子命令。
///
/// 日志必须在 `run` 之前初始化，且返回的 `WorkerGuard` 要活到 `run` 结束：守卫一旦
/// 析构，非阻塞写线程就会被关闭，尾部日志可能丢失。两条分支都用局部变量接住返回值，
/// 正是为了让它在 `run` 返回之后才析构。
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Search(args) => {
            let _log_guard = logging::init(args.verbose, args.disable_log_file, "search-rust.log");
            search::run(args).await
        }
        Command::Hilog(args) => {
            let _log_guard = logging::init(args.verbose, args.disable_log_file, "hilog-rust.log");
            hilog::run(args).await
        }
    }
}
