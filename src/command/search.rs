//! `search` 子命令：为每台在线设备并行启动 AppGallery 搜索流程。
//!
//! 本模块只做「参数 → 设备」的翻译和失败汇总，UI 流程见
//! [`crate::search::flow::run_device`]：收集分类应用名 → 逐个搜索并收集同开发者应用
//! → 刷新分类后重搜漏掉的名称。
//!
//! 失败聚合策略：所有在线设备同时启动，任一台失败都不会取消其他设备；等全部结束后
//! 把每台的错误用 ` | ` 拼成一条错误返回，因此一次执行可能同时看到多台设备的失败原因。

use anyhow::{Context, Result, bail};
use clap::Args;
use hm_driver_rs::{DeviceStatus, HdcConfig, HmDriver};
use std::path::PathBuf;
use tokio::task::JoinSet;
use tracing::info;

use crate::search::flow::run_device;

/// `search` 子命令的参数；除日志与 HDC 路径之外的开关都会传给每台设备的搜索流程。
#[derive(Clone, Debug, Args)]
pub struct SearchArgs {
    /// 丢弃已保存的进度，从头开始。
    ///
    /// 进度按设备序列号保存在 `.cache/search/` 下（`--random` 模式用 `.random.json`
    /// 后缀，两种模式互不影响）；该开关会立刻把状态文件改写为空进度，但不会删除文件。
    #[arg(long)]
    fresh: bool,

    /// 只收集新鲜应用，并按随机顺序搜索。
    ///
    /// 开启后只遍历「应用」页（跳过「游戏」），并在每个分类里进入「新鲜应用」入口；
    /// 待搜索名称也会被打乱顺序。
    #[arg(long)]
    random: bool,

    /// 按 hilog 的深度遍历分类：进入分类后逐个走子分类入口，并把应用列表滑到底。
    ///
    /// 默认（不加该开关）只读分类首页固定 3 屏。打开后走「新鲜应用/新鲜游戏/时下畅销应用/
    /// 时下畅销游戏」子分类页，每个列表滑到底，深度与 `hilog` 子命令一致。与 `--random`
    /// 独立：`--random` 决定页签范围与搜索顺序，爬取深度由本开关决定。
    ///
    /// 只影响真正执行的遍历：进度里已有 `collection_complete` 时会跳过初始遍历，配 `--fresh`
    /// 才会重爬。
    #[arg(long)]
    deep: bool,

    /// 跳过「打开搜索结果里的第一个应用，收集同开发者的应用」这一步。
    ///
    /// 对应 `run_device` 的 `developer_scan` 参数：关掉之后只验证搜索是否有结果，
    /// 因此更快，但不会再补充同开发者应用名。
    #[arg(long)]
    skip_developer: bool,

    /// 输出更详细的日志。
    ///
    /// 打开后日志级别由 `INFO` 提升到 `DEBUG`（见 [`crate::logging::init`]）。
    #[arg(short, long)]
    pub verbose: bool,

    /// 不写入日志文件。
    ///
    /// 默认按日写入 `logs/search-rust.log.<日期>`；打开后只保留终端输出。
    #[arg(long)]
    pub disable_log_file: bool,

    /// 指定 HDC 可执行文件路径。
    ///
    /// 不指定时用默认 `HdcConfig`（路径为空，运行时从 `HDC_PATH` / `PATH` 推导）。
    #[arg(long)]
    hdc_path: Option<PathBuf>,
}

/// 发现设备、过滤出在线设备，并在所有在线设备上并行执行搜索流程。
///
/// 没有在线设备时直接失败；只要有设备在线就全部启动，等全部结束再汇总失败，
/// 因此单台设备的报错不会掩盖其他设备的结果。
pub async fn run(cli: SearchArgs) -> Result<()> {
    let hdc_config = cli
        .hdc_path
        .clone()
        .map_or_else(HdcConfig::default, |path| {
            HdcConfig::default().with_path(path)
        });
    let descriptors = HmDriver::discover_devices(hdc_config.clone())
        .await
        .context("发现设备失败")?;
    // 离线 / 未授权 / 未知状态的设备无法建立会话，直接跳过，而不是让整批设备一起失败。
    let online: Vec<String> = descriptors
        .into_iter()
        .filter_map(|descriptor| {
            if descriptor.status == DeviceStatus::Online {
                Some(descriptor.serial.expose_secret().to_owned())
            } else {
                None
            }
        })
        .collect();
    if online.is_empty() {
        bail!("未发现在线 HarmonyOS 设备");
    }

    info!(devices = online.len(), "发现在线 HarmonyOS 设备");
    for (index, serial) in online.iter().enumerate() {
        info!(device = format_args!("device-{index}"), serial, "发现设备");
    }

    // 各设备的进度按序列号分文件保存，彼此不共享可变状态，因此可以直接全部并发，
    // 不需要额外的互斥或限流。
    let mut tasks = JoinSet::new();
    for (index, serial) in online.into_iter().enumerate() {
        tasks.spawn(run_device(
            index,
            serial,
            hdc_config.clone(),
            cli.fresh,
            cli.random,
            cli.deep,
            !cli.skip_developer,
        ));
    }

    // 收完所有设备再统一上报：先失败的任务不会取消仍在运行的设备。
    let mut failures = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => failures.push(format!("{error:?}")),
            Err(error) => failures.push(format!("设备任务异常：{error}")),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        bail!(
            "{} 台设备执行失败：{}",
            failures.len(),
            failures.join(" | ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// 只为了让 `SearchArgs` 能被单独解析。
    #[derive(Debug, Parser)]
    struct TestCli {
        #[command(flatten)]
        args: SearchArgs,
    }

    /// `--deep` 是可选开关，默认必须保持关闭（默认仍是「只读分类首页 3 屏」）。
    #[test]
    fn deep_默认关闭() {
        let cli = TestCli::parse_from(["test"]);
        assert!(!cli.args.deep);
        assert!(!cli.args.random);
    }

    #[test]
    fn deep_与_random_可以同时打开() {
        let cli = TestCli::parse_from(["test", "--deep", "--random"]);
        assert!(cli.args.deep);
        assert!(cli.args.random);
    }
}
