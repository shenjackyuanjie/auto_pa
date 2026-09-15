//! Rust 二进制共用的 AppGallery UI 与搜索模块。
//!
//! `main.rs` 只做命令行解析，实际能力都来自这里：`command` 负责设备发现与并发调度，
//! `appgallery` 提供与 AppGallery 界面相关的常量与控件解析，`hilog` 和 `search`
//! 是两个子命令各自的业务流程，`logging` 统一终端与文件日志。

/// AppGallery 的 bundle/ability 常量，以及分类按钮和应用卡片的界面解析。
pub mod appgallery;
/// `auto-pa` 的子命令参数、设备发现与并发调度。
pub mod command;
/// `hilog` 子命令的纯 UI 分类遍历流程。
pub mod hilog;
/// 终端与按日滚动日志文件的 tracing 初始化。
pub mod logging;
/// `search` 子命令的名称收集、进度保存与搜索流程。
pub mod search;
