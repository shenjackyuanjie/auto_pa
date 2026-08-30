use anyhow::{Context, Result, bail};
use clap::Args;
use hm_driver_rs::{DeviceSelector, DeviceSerial, DeviceStatus, HdcConfig, HmDriver};
use std::path::PathBuf;
use std::time::Duration;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tracing::{error, info};

use crate::hilog::{UiTraversal, UiTraversalConfig};

#[derive(Clone, Debug, Args)]
pub struct HilogArgs {
    /// 显示更详细的日志。
    #[arg(short, long)]
    pub verbose: bool,

    /// 不写入日志文件。
    #[arg(long)]
    pub disable_log_file: bool,

    /// 指定 HDC 可执行文件路径。
    #[arg(long)]
    hdc_path: Option<PathBuf>,

    /// 跳过指定的分类名称。
    #[arg(long, num_args = 1.., value_name = "分类")]
    skip_categories: Vec<String>,

    /// 执行轮数。
    #[arg(long = "loop", default_value_t = 1)]
    loop_count: usize,

    /// 两轮之间的等待时间，支持 5m、1h30m、00h00m05s 或纯秒数。
    #[arg(long, default_value = "5m", value_parser = parse_duration)]
    loop_wait: Duration,

    /// 与 Python 实现一致的 UI 等待参数；15 表示约 1.75 秒。
    #[arg(long, default_value_t = 15)]
    ping: u64,

    /// 设备执行失败时不关闭 AppGallery，便于排查 UI 状态。
    #[arg(long)]
    keep_open_on_error: bool,

    /// 启用 hilog 抓取和应用投稿。该完整流程尚未实现，当前会明确报错。
    #[arg(long)]
    submit: bool,

    /// 投稿用户名；仅可与 --submit 一起使用。
    #[arg(long)]
    username: Option<String>,
}

async fn run_device(
    index: usize,
    serial: String,
    hdc_config: HdcConfig,
    cli: HilogArgs,
) -> Result<()> {
    let device_label = format!("device-{index}");
    info!(device = %device_label, serial = %serial, "开始处理设备");
    let driver = HmDriver::builder()
        .device(DeviceSelector::Serial(DeviceSerial::new(serial)))
        .hdc_config(hdc_config)
        .connect()
        .await
        .context("连接 HmDriver 失败")?;
    let config = UiTraversalConfig::new(cli.skip_categories.clone(), cli.ping);
    let mut flow = UiTraversal::new(driver, config, device_label.clone())?;
    let run_result = flow.run().await;
    let cleanup_result = flow
        .shutdown(!(run_result.is_err() && cli.keep_open_on_error))
        .await;

    match (run_result, cleanup_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(error.context(format!("同时清理失败：{cleanup}"))),
    }
}

async fn run_round(cli: &HilogArgs, hdc_config: &HdcConfig) -> Result<()> {
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
    let mut tasks = JoinSet::new();
    for (index, serial) in online.into_iter().enumerate() {
        tasks.spawn(run_device(index, serial, hdc_config.clone(), cli.clone()));
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

pub async fn run(cli: HilogArgs) -> Result<()> {
    if cli.loop_count == 0 {
        bail!("--loop 必须大于 0");
    }
    if cli.username.is_some() && !cli.submit {
        bail!("--username 仅能与 --submit 一起使用");
    }
    if cli.submit {
        let username = cli.username.as_deref().unwrap_or_default().trim();
        if username.is_empty() {
            bail!("启用 --submit 时必须提供 --username");
        }
        bail!("--submit 的 hilog 抓取和应用投稿流程尚未实现；请不要假定已产生投稿");
    }

    let hdc_config = cli
        .hdc_path
        .clone()
        .map_or_else(HdcConfig::default, |path| {
            HdcConfig::default().with_path(path)
        });
    let mut last_error = None;
    for round in 0..cli.loop_count {
        info!(
            round = round + 1,
            total = cli.loop_count,
            "开始 UI 遍历轮次"
        );
        match run_round(&cli, &hdc_config).await {
            Ok(()) => last_error = None,
            Err(error) => {
                error!(round = round + 1, error = %error, "UI 遍历轮次失败");
                last_error = Some(error);
            }
        }
        if round + 1 < cli.loop_count {
            info!(wait = ?cli.loop_wait, "等待下一轮");
            sleep(cli.loop_wait).await;
        }
    }

    match last_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn parse_duration(value: &str) -> std::result::Result<Duration, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("等待时间不能为空".to_owned());
    }
    if let Ok(seconds) = value.parse::<f64>() {
        return duration_from_seconds(seconds);
    }

    let bytes = value.as_bytes();
    let mut index = 0usize;
    let mut seconds = 0.0f64;
    while index < bytes.len() {
        let start = index;
        let mut has_dot = false;
        while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'.') {
            if bytes[index] == b'.' {
                if has_dot {
                    return Err(format!("等待时间格式无效：{value}"));
                }
                has_dot = true;
            }
            index += 1;
        }
        if start == index || index == bytes.len() {
            return Err(format!("等待时间格式无效：{value}"));
        }
        let number = value[start..index]
            .parse::<f64>()
            .map_err(|_| format!("等待时间格式无效：{value}"))?;
        let factor = match bytes[index] {
            b'h' => 3_600.0,
            b'm' => 60.0,
            b's' => 1.0,
            _ => return Err(format!("等待时间单位无效：{value}")),
        };
        seconds += number * factor;
        index += 1;
    }
    duration_from_seconds(seconds)
}

fn duration_from_seconds(seconds: f64) -> std::result::Result<Duration, String> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err("等待时间必须是非负有限数值".to_owned());
    }
    Ok(Duration::from_secs_f64(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 能解析等待时间() {
        assert_eq!(parse_duration("5m"), Ok(Duration::from_secs(300)));
        assert_eq!(parse_duration("00h00m05s"), Ok(Duration::from_secs(5)));
        assert_eq!(parse_duration("1.5s"), Ok(Duration::from_millis(1_500)));
        assert!(parse_duration("5x").is_err());
    }
}
