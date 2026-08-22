use anyhow::{Context, Result, bail};
use clap::Args;
use hm_driver_rs::{DeviceStatus, HdcConfig, HmDriver};
use std::path::PathBuf;
use tokio::task::JoinSet;
use tracing::info;

use crate::search::flow::run_device;

#[derive(Clone, Debug, Args)]
pub struct SearchArgs {
    /// 丢弃已保存的进度，从头开始。
    #[arg(long)]
    fresh: bool,

    /// 只收集新鲜应用，并按随机顺序搜索。
    #[arg(long)]
    random: bool,

    /// 输出更详细的日志。
    #[arg(short, long)]
    pub verbose: bool,

    /// 不写入日志文件。
    #[arg(long)]
    pub disable_log_file: bool,

    /// 指定 HDC 可执行文件路径。
    #[arg(long)]
    hdc_path: Option<PathBuf>,
}

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

    let mut tasks = JoinSet::new();
    for (index, serial) in online.into_iter().enumerate() {
        tasks.spawn(run_device(
            index,
            serial,
            hdc_config.clone(),
            cli.fresh,
            cli.random,
        ));
    }

    let mut failures = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => failures.push(error.to_string()),
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
