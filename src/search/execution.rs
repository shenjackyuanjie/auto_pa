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
const SEARCH_RESULT_BACK_KEY: &str = "SearchInputCard.Button.searchFrameBack";
const MAX_SEARCH_ATTEMPTS: usize = 3;
const SEARCH_INPUT_FOCUS_SETTLE: Duration = Duration::from_millis(200);
const SEARCH_INPUT_SETTLE: Duration = Duration::from_millis(300);
const SEARCH_CLICK_SETTLE: Duration = Duration::from_millis(100);

impl SearchFlow {
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
                        warn!(app = %app_name, attempt, error = %error, "搜索失败");
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
        if is_english_query(app_name) {
            debug!(app = %app_name, "英文查询输入完成，收起输入法");
            self.driver.press_key_code(KeyCode::Enter).await?;
            sleep(SEARCH_CLICK_SETTLE).await;
        }

        self.click_key_prefix(SEARCH_BUTTON_KEY_PREFIX, Duration::from_secs(5), "搜索按钮")
            .await?;
        sleep(SEARCH_CLICK_SETTLE).await;

        let result_page = self
            .wait_local_key(
                SEARCH_RESULT_BACK_KEY,
                Duration::from_secs(15),
                "搜索结果页",
            )
            .await?;
        let result_count = match self
            .driver
            .wait_for_ui_tree(Duration::from_secs(6), |tree| {
                !app_snapshot(tree).is_empty()
            })
            .await
        {
            Ok(layout) => self.collect_app_list(layout, false).await?.len(),
            Err(_) => {
                debug!(app = %app_name, "搜索结果页没有应用卡片");
                let _ = result_page;
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
