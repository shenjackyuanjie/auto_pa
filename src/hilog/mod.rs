//! `hilog` 子命令的纯 UI 流程：在每台在线设备上启动 AppGallery，遍历「应用」和
//! 「游戏」的分类页面，并把每个应用列表下滑至稳定。
//!
//! 它是 Python 命令 `uv run .\main.py hilog --no-submit` 的 UI 部分。当前版本不抓取
//! hilog、也不提交应用，`--submit` 会被 `command::hilog::run` 直接拒绝，避免悄悄执行
//! 不完整的投稿流程。
//!
//! 与 [`crate::search`] 的差别：search 会把应用名写入进度文件、逐个搜索，并按参数进入
//! 「新鲜应用」或「同开发者的应用」列表；这里没有任何状态持久化，只关心界面能否走通，
//! 因此偶发问题只记录 warning 并跳过当前分类。

/// 遍历实现；对外只暴露下面的两个类型。
mod traversal;

pub use traversal::{UiTraversal, UiTraversalConfig};
