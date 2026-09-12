//! `search` 子命令的业务流程：先遍历分类收集应用名称并持久化进度，再逐个搜索这些
//! 名称，最后刷新一轮分类以搜索新发现的名字。
//!
//! 子模块的可见性区分了两类内容：`flow` 与 `state` 是 `pub`（`command` 层从这里获取
//! 设备流程入口，进度文件的格式也由此对外暴露）；`collection`、`developer`、
//! `execution` 只是分别给 [`SearchFlow`](flow::SearchFlow) 挂 `impl` 的内部实现，
//! 属于实现细节，保持私有。

/// 搜索主流程：启动 AppGallery、收集分类名称、按进度搜索并处理失败重试。
pub mod flow;
/// 每台设备的进度文件与已收集/已搜索名称状态。
pub mod state;

/// 分类页面里的应用名称收集实现。
mod collection;
/// 应用详情页「同开发者的应用」收集实现。
mod developer;
/// 搜索框输入、搜索结果页读取与开发者收集的编排实现。
mod execution;
