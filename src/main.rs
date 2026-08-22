use anyhow::Result;
use auto_pa_rs::command::{hilog, search};
use auto_pa_rs::logging;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "auto-pa", about = "AppGallery 自动化工具")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 收集 AppGallery 分类应用名称并逐个搜索。
    Search(search::SearchArgs),
    /// 遍历 AppGallery 分类和应用列表；默认不抓取 hilog、不提交应用。
    Hilog(hilog::HilogArgs),
}

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
