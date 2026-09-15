//! 按名称搜索 AppGallery：把收集到的应用名逐个填进搜索框并提交。
//!
//! 该模块是 `SearchFlow::run` 的第二阶段：`collection` 先遍历分类把应用名写进状态文件，
//! 这里再按名称逐个搜索，把结果页的第一个应用交给 `developer` 做同开发者的应用收集。
//!
//! 前置页面状态：AppGallery 已启动并停留在「应用」页签的搜索首页。`ensure_search_home`
//! 负责建立这个状态，`search_once` 结束时也必须回到同一状态，下一轮名称才能直接继续输入。
//! 本轮失败的名称不会被标记为已搜索，由调用方在下一轮重试。

use anyhow::{Context, Result, bail};
use hm_driver_rs::{KeyCode, UiNode};
use rand::seq::SliceRandom;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use crate::appgallery::app_snapshot;
use crate::search::flow::SearchFlow;

const SEARCH_FIELD_KEY_PREFIX: &str = "__SearchField__search_box";
const SEARCH_BUTTON_KEY_PREFIX: &str = "__SearchField__Button__search_box";
pub(crate) const SEARCH_RESULT_BACK_KEY: &str = "SearchInputCard.Button.searchFrameBack";
const MAX_SEARCH_ATTEMPTS: usize = 3;
const SEARCH_INPUT_FOCUS_SETTLE: Duration = Duration::from_millis(200);
const SEARCH_INPUT_SETTLE: Duration = Duration::from_millis(300);
const SEARCH_CLICK_SETTLE: Duration = Duration::from_millis(100);
const SEARCH_BUTTON_TIMEOUT: Duration = Duration::from_secs(5);
const RESULT_LIST_TIMEOUT: Duration = Duration::from_secs(6);
/// 回车提交后浮层可能连同“搜索”按钮一起消失，因此只做一次短暂探测。
const SEARCH_BUTTON_AFTER_ENTER_TIMEOUT: Duration = Duration::from_millis(800);

impl SearchFlow {
    /// 搜索所有尚未搜索过的名称，任一名称三次尝试都失败则整轮报错。
    ///
    /// 单次失败会先把 `home_ready` 置为假再重建搜索首页，因为失败可能发生在任何一步，
    /// 当前页面已经不确定；重建比猜测界面状态更可靠。已经成功的名称逐个落盘，中途中断也
    /// 不会丢失进度。
    pub(crate) async fn search_pending(&mut self) -> Result<()> {
        let mut pending = self.state.pending_names();
        if pending.is_empty() {
            info!(device = %self.device_label, "所有应用名称均已搜索完成");
            return Ok(());
        }
        if self.random_mode {
            pending.shuffle(&mut rand::rng());
        }

        self.ensure_search_home().await?;
        let total = pending.len();
        info!(device = %self.device_label, pending = total, "开始搜索应用名称");
        let mut failed = Vec::new();
        for (index, app_name) in pending.into_iter().enumerate() {
            let mut success = false;
            for attempt in 1..=MAX_SEARCH_ATTEMPTS {
                info!(
                    device = %self.device_label,
                    app = %app_name,
                    progress = format_args!("{}/{}", index + 1, total),
                    attempt,
                    "搜索应用"
                );
                match self.search_once(&app_name).await {
                    Ok(result_count) => {
                        self.state.mark_searched(app_name.clone());
                        self.store.save(&self.state)?;
                        info!(app = %app_name, results = result_count, "搜索完成");
                        success = true;
                        break;
                    }
                    Err(error) => {
                        warn!(app = %app_name, attempt, error = ?error, "搜索失败");
                        self.home_ready = false;
                        if attempt < MAX_SEARCH_ATTEMPTS {
                            self.ensure_search_home().await?;
                        }
                    }
                }
            }
            if !success {
                failed.push(app_name);
            }
        }

        if failed.is_empty() {
            info!(device = %self.device_label, "本轮应用搜索全部完成");
            Ok(())
        } else {
            error!(
                device = %self.device_label,
                failed = failed.len(),
                names = %failed.join(", "),
                "本轮应用搜索存在失败项"
            );
            bail!(
                "有 {} 个应用搜索失败，下次运行会重试：{}",
                failed.len(),
                failed.join(", ")
            )
        }
    }

    /// 确保停在搜索首页：重启 AppGallery、进入「应用」页签并等到搜索框出现。
    ///
    /// `home_ready` 只是缓存：搜索过程中的失败或页面跳转都会把它置为假，避免在错误页面上
    /// 继续按 key 等待搜索控件。
    async fn ensure_search_home(&mut self) -> Result<()> {
        if self.home_ready {
            return Ok(());
        }
        info!(device = %self.device_label, "准备应用搜索主页");
        self.start_appgallery().await?;
        self.click_app_or_game("应用").await?;
        self.wait_for_key_node(SEARCH_FIELD_KEY_PREFIX, Duration::from_secs(12))
            .await?;
        self.home_ready = true;
        info!(device = %self.device_label, "应用搜索主页就绪");
        Ok(())
    }

    /// 搜索单个名称并返回搜索结果页第一屏的应用数量。
    ///
    /// 流程：点击搜索输入框并输入文本、提交搜索、在搜索结果页上做开发者收集与列表统计，
    /// 最后点结果页返回按钮回到搜索首页。成功的唯一判据是结果页返回按钮出现，搜索按钮是否
    /// 存在不参与判断。调用方需要 `ensure_search_home` 建立的前置状态；成功返回时页面停在
    /// 搜索首页（`home_ready` 保持有效），中途报错则由调用方重建首页。
    async fn search_once(&mut self, app_name: &str) -> Result<usize> {
        self.click_key_prefix(
            SEARCH_FIELD_KEY_PREFIX,
            Duration::from_secs(12),
            "搜索输入框",
        )
        .await?;
        sleep(SEARCH_INPUT_FOCUS_SETTLE).await;
        self.driver.input_text(app_name).await?;
        sleep(SEARCH_INPUT_SETTLE).await;

        // 提交搜索：英文查询先按回车，其余情况点“搜索”按钮。
        //
        // 部分设备（如 MatePad）按回车会直接提交并跳转搜索结果页，此时搜索框与浮层里的
        // “搜索”按钮的 key 从 `…search_boxN` 变成 `…searchFrameInput1`，按钮随即消失。
        // 因此回车之后按钮缺失属于正常情况，不能当作搜索失败：本次搜索是否成功，交给
        // 后面的搜索结果页判断。
        let english = is_english_query(app_name);
        if english {
            debug!(app = %app_name, "英文查询输入完成，按回车提交");
            self.driver.press_key_code(KeyCode::Enter).await?;
            sleep(SEARCH_CLICK_SETTLE).await;
        }

        // 中文等非英文查询不会走回车路径，搜索按钮应当还在，因此照常等待完整超时。
        let button_timeout = if english {
            SEARCH_BUTTON_AFTER_ENTER_TIMEOUT
        } else {
            SEARCH_BUTTON_TIMEOUT
        };
        match self
            .click_key_prefix(SEARCH_BUTTON_KEY_PREFIX, button_timeout, "搜索按钮")
            .await
        {
            Ok(()) => sleep(SEARCH_CLICK_SETTLE).await,
            Err(error) if english => {
                debug!(app = %app_name, error = ?error, "回车后未找到搜索按钮，按已提交处理");
            }
            Err(error) => return Err(error),
        }

        self.wait_local_key(
            SEARCH_RESULT_BACK_KEY,
            Duration::from_secs(15),
            "搜索结果页",
        )
        .await?;

        // 结果页刚打开时列表还在顶部，先取第一个应用做开发者收集：`collect_app_list`
        // 会把列表滚到半路，之后再想点第一个应用就得先滚回顶部。
        //
        // 这里只用 `.ok()`：跳过开发者收集、或结果页确实没有卡片时，搜索本身仍算成功。
        let result_layout = self
            .driver
            .wait_for_ui_tree(RESULT_LIST_TIMEOUT, |tree| !app_snapshot(tree).is_empty())
            .await
            .ok();

        if self.developer_scan
            && let Some(layout) = result_layout.as_ref()
        {
            match self.collect_developer_apps(layout).await {
                Ok(collected) => {
                    info!(app = %app_name, collected, "同开发者应用收集结束");
                }
                Err(error) => {
                    warn!(app = %app_name, error = ?error, "同开发者应用收集失败");
                    // 收集过程中可能已经离开结果页，先恢复页面再继续统计结果列表。
                    self.back_to_result_page().await?;
                }
            }
        }

        let result_count = match result_layout {
            Some(layout) => self.collect_app_list(layout, false).await?.len(),
            None => {
                debug!(app = %app_name, "搜索结果页没有应用卡片");
                0
            }
        };

        self.click_local_key(
            SEARCH_RESULT_BACK_KEY,
            Duration::from_secs(8),
            "搜索结果页返回按钮",
        )
        .await?;
        sleep(SEARCH_CLICK_SETTLE).await;
        self.wait_for_key_node(SEARCH_FIELD_KEY_PREFIX, Duration::from_secs(15))
            .await?;
        self.home_ready = true;
        Ok(result_count)
    }

    /// 使用 `dumpLayout` 的本地快照按 key 等待控件。
    ///
    /// 部分 Hypium Agent 未实现 `On.key`，不能使用远端 `Selector::key`；但布局树仍会
    /// 暴露 key 和 bounds，因此在本地匹配后按坐标操作。
    async fn wait_for_key_node(&self, key_prefix: &str, timeout: Duration) -> Result<UiNode> {
        self.driver
            .wait_for_ui(timeout, |node| key_starts_with(node, key_prefix))
            .await
            .with_context(|| format!("等待控件 [{}] 超时", key_prefix))
    }

    async fn wait_local_key(
        &self,
        key: &str,
        timeout: Duration,
        description: &str,
    ) -> Result<UiNode> {
        self.driver
            .wait_for_ui(timeout, |node| node.attribute_str("key") == Some(key))
            .await
            .with_context(|| format!("等待 [{}] 超时", description))
    }

    async fn click_key_prefix(
        &self,
        key_prefix: &str,
        timeout: Duration,
        description: &str,
    ) -> Result<()> {
        let node = self.wait_for_key_node(key_prefix, timeout).await?;
        self.click_ui_node(node, description).await
    }

    async fn click_local_key(&self, key: &str, timeout: Duration, description: &str) -> Result<()> {
        let node = self.wait_local_key(key, timeout, description).await?;
        self.click_ui_node(node, description).await
    }

    async fn click_ui_node(&self, node: UiNode, description: &str) -> Result<()> {
        let bounds = node
            .bounds()
            .ok_or_else(|| anyhow::anyhow!("控件 [{description}] 没有有效 bounds"))?;
        self.driver
            .click(bounds.center())
            .await
            .with_context(|| format!("点击控件 [{description}] 失败"))
    }
}

fn key_starts_with(node: &UiNode, key_prefix: &str) -> bool {
    node.attribute_str("key")
        .is_some_and(|key| key.starts_with(key_prefix))
}

/// 判断查询是否能用软键盘回车提交：必须全是 ASCII 且含有英文字母。
///
/// 纯数字查询按回车没有提交效果，所以要求至少一个 ASCII 字母。
fn is_english_query(value: &str) -> bool {
    value
        .chars()
        .any(|character| character.is_ascii_alphabetic())
        && value.is_ascii()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn 英文查询识别符合键盘处理逻辑() {
        assert!(is_english_query("Facebook Lite 2.0"));
        assert!(!is_english_query("微信"));
        assert!(!is_english_query("Facebook 微信"));
        assert!(!is_english_query("12345"));
    }

    #[test]
    fn key_以前缀匹配布局节点() {
        let node: UiNode = serde_json::from_value(json!({
            "attributes": {"key": "__SearchField__search_box2"},
            "children": []
        }))
        .unwrap();
        assert!(key_starts_with(&node, SEARCH_FIELD_KEY_PREFIX));
        assert!(!key_starts_with(&node, SEARCH_BUTTON_KEY_PREFIX));
    }
}
