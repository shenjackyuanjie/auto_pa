//! `hilog` 子命令：按轮次在每台在线设备上遍历 AppGallery 分类与应用列表。
//!
//! 默认只做 UI 遍历，不抓取 hilog、也不投稿；`--submit` 的完整流程尚未实现，
//! 启用它会在校验阶段就报错，避免让人误以为已经产生投稿。
//!
//! 轮次语义：每轮重新发现设备并并行执行，单轮失败只记录日志、不中止后续轮次，
//! 循环结束后返回**最后一轮**的错误（所有轮次都成功才返回 `Ok`）。

use anyhow::{Context, Result, bail};
use clap::Args;
use hm_driver_rs::{DeviceSelector, DeviceSerial, DeviceStatus, HdcConfig, HmDriver};
use std::path::PathBuf;
use std::time::Duration;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tracing::{error, info};

use crate::hilog::{UiTraversal, UiTraversalConfig};

/// `hilog` 子命令的参数；除日志与 HDC 路径之外的开关都会传给每台设备的遍历流程。
#[derive(Clone, Debug, Args)]
pub struct HilogArgs {
    /// 显示更详细的日志。
    ///
    /// 打开后日志级别由 `INFO` 提升到 `DEBUG`（见 [`crate::logging::init`]）。
    #[arg(short, long)]
    pub verbose: bool,

    /// 不写入日志文件。
    ///
    /// 默认按日写入 `logs/hilog-rust.log.<日期>`；打开后只保留终端输出。
    #[arg(long)]
    pub disable_log_file: bool,

    /// 指定 HDC 可执行文件路径。
    ///
    /// 不指定时用默认 `HdcConfig`（路径为空，运行时从 `HDC_PATH` / `PATH` 推导）。
    #[arg(long)]
    hdc_path: Option<PathBuf>,

    /// 跳过指定的分类名称。
    ///
    /// 名称需与分类按钮上的文本完全一致；可以一次给出多个值，重复项按集合去重，
    /// 被跳过的分类不会进入其应用列表。
    #[arg(long, num_args = 1.., value_name = "分类")]
    skip_categories: Vec<String>,

    /// 执行轮数。
    ///
    /// 参数名是 `--loop`，必须大于 0，否则在发现设备前就报错。
    #[arg(long = "loop", default_value_t = 1)]
    loop_count: usize,

    /// 两轮之间的等待时间，支持 5m、1h30m、00h00m05s 或纯秒数。
    ///
    /// 只在还有下一轮时才等待；解析规则见 `parse_duration`。
    #[arg(long, default_value = "5m", value_parser = parse_duration)]
    loop_wait: Duration,

    /// UI 等待参数；15 表示约 1.75 秒。
    ///
    /// 实际换算成分类点击后的固定等待 `1.0 + ping * 0.05` 秒。
    #[arg(long, default_value_t = 15)]
    ping: u64,

    /// 设备执行失败时不关闭 AppGallery，便于排查 UI 状态。
    ///
    /// 只影响是否停止 AppGallery；无论成败都会关闭 HDC 连接（见 `run_device`）。
    #[arg(long)]
    keep_open_on_error: bool,

    /// 启用 hilog 抓取和应用投稿。该完整流程尚未实现，当前会明确报错。
    ///
    /// 还需要同时提供 `--username`；两者都满足时依然会以「尚未实现」失败，
    /// 因此不能把它当作可用的投稿开关。
    #[arg(long)]
    submit: bool,

    /// 投稿用户名；仅可与 --submit 一起使用。
    ///
    /// 单独给出会被拒绝；与 `--submit` 同时给出时会去掉首尾空白后判空。
    #[arg(long)]
    username: Option<String>,
}

/// 在单台设备上执行一轮 UI 遍历，并保证清理逻辑一定执行。
///
/// `keep_open_on_error` 为真且遍历失败时保留 AppGallery 现场（即 `shutdown(false)`），
/// 否则照常关闭应用；HDC 连接始终关闭。返回值以遍历错误为主，若清理也失败则把它
/// 作为上下文附加到同一个错误上，避免清理错误掩盖真正的失败原因。
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
    // 只有「遍历失败 + 用户要求保留现场」才不关应用，其余情况（含成功）都照常清理。
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

/// 执行一轮遍历：每轮都重新发现设备，再在所有在线设备上并行遍历。
///
/// 与 `search` 一致：非 `Online` 的设备被跳过，没有在线设备直接失败；只要有设备在线就
/// 全部启动，等全部结束再把各设备的失败原因用 ` | ` 拼成一条错误返回。
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

/// 校验参数后按 `--loop` 轮次执行 UI 遍历。
///
/// 三项校验都在发现设备之前完成：`--loop` 必须大于 0、`--username` 不能脱离 `--submit`
/// 单独使用、`--submit` 必须带 `--username`。每轮失败只记日志并继续，轮间等待
/// `--loop-wait`，最后返回最后一次失败的错误。
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

/// 把 `--loop-wait` 解析成等待时长：支持纯秒数（含小数）以及 `h` / `m` / `s` 单位的组合。
///
/// 返回 `Err(String)` 而不是结构化错误，是为了交给 clap 直接作为「参数值非法」展示；
/// 负数与非有限值（如 `inf`）统一由 [`duration_from_seconds`] 拒绝。
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

/// 把秒数转换为 [`Duration`]，并拒绝负数与非有限值。
///
/// 纯秒数与带单位两种写法最终都汇到这里，因此这里是唯一的取值边界。
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
