//! `hilog` 的 UI 遍历实现：启动 AppGallery，进入「应用」/「游戏」的分类页，逐层
//! 走完分类、子分类下的每个应用列表。
//!
//! 这里不保存任何结果，应用名称只用于判断列表是否还有新内容。
//!
//! 容错策略：偶发问题不升级为设备失败，
//!
//! - 等待分类内容或应用列表超时只记 warning，退化为当前 UI，由调用方按空内容跳过；
//! - 当前页面没有应用卡片时同样只记 warning，结束该列表的读取并返回已收集数量；
//! - 只有结构性失败才报错：等不到 AppGallery 前台、找不到分类按钮、滚动次数超过上限。

use anyhow::{Context, Result, anyhow, bail};
use hm_driver_rs::{AppIdentifier, HmDriver, SwipeArea, SwipeDirection, UiNode};
use std::collections::HashSet;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::appgallery::{
    APPGALLERY_ABILITY, APPGALLERY_BUNDLE, CategoryButton, app_snapshot, category_buttons,
};
use crate::logging::error_chain;

/// 单个应用列表允许的最大下滑次数，超过即视为结构性失败。
const MAX_SCROLLS: usize = 100;
/// 分类列表和子分类列表允许的最大下滑次数，超过即视为结构性失败。
const MAX_CATEGORY_SCROLLS: usize = 100;

// 各步骤的固定等待时间，取值与 search 流程保持一致；与 `--ping` 相关的分类点击等待
// 由 `UiTraversalConfig` 换算。
const APP_STOP_SETTLE: Duration = Duration::from_secs(1);
const APP_START_SETTLE: Duration = Duration::from_secs(3);
const PAGE_CLICK_SETTLE: Duration = Duration::from_millis(750);
const CATEGORY_SCROLL_SETTLE: Duration = Duration::from_millis(850);
const BACK_SETTLE: Duration = Duration::from_millis(1500);
const LIST_SCROLL_SETTLE: Duration = Duration::from_millis(100);

/// 遍历的行为参数，由命令行参数换算而来。
pub struct UiTraversalConfig {
    /// 跳过的分类名称，命中后只记日志、不进入。
    skipped_categories: HashSet<String>,
    /// 点击分类后的等待时间。
    category_click_settle: Duration,
}

impl UiTraversalConfig {
    /// 由 `--skip-categories` 和 `--ping` 构造参数。
    ///
    /// `ping` 的换算：每 1 点约 0.05 秒，默认 15 对应约 1.75 秒的分类点击
    /// 等待。
    pub fn new(skip_categories: Vec<String>, ping: u64) -> Self {
        Self {
            skipped_categories: skip_categories.into_iter().collect(),
            category_click_settle: Duration::from_secs_f64(1.0 + ping as f64 * 0.05),
        }
    }
}

/// 单台设备的 UI 遍历流程，持有调用方已连接的驱动和设备标签。
pub struct UiTraversal {
    driver: HmDriver,
    bundle: AppIdentifier,
    device_label: String,
    config: UiTraversalConfig,
}

impl UiTraversal {
    /// 绑定 AppGallery 的 bundle 后接管驱动；驱动连接由调用方完成。
    pub fn new(driver: HmDriver, config: UiTraversalConfig, device_label: String) -> Result<Self> {
        Ok(Self {
            driver,
            bundle: AppIdentifier::new(APPGALLERY_BUNDLE)?,
            device_label,
            config,
        })
    }

    /// 启动 AppGallery，然后依次遍历「应用」和「游戏」两个分类页面。
    ///
    /// 全程只做 UI 遍历：不抓取 hilog，也不提交应用。
    pub async fn run(&mut self) -> Result<()> {
        self.start_appgallery().await?;
        for page in ["应用", "游戏"] {
            self.traverse_page(page).await?;
        }
        Ok(())
    }

    /// 关闭 AppGallery（`close_app` 为 false 时保留现场）并释放驱动。
    ///
    /// 两步的失败会合并汇报，避免清理阶段的问题互相掩盖。
    pub async fn shutdown(&self, close_app: bool) -> Result<()> {
        let mut failures = Vec::new();
        if close_app {
            info!(device = %self.device_label, "关闭 AppGallery");
            if let Err(error) = self.driver.stop_app(&self.bundle).await {
                failures.push(format!("关闭 AppGallery 失败：{}", error_chain(&error)));
            }
        } else {
            info!(device = %self.device_label, "保留 AppGallery 现场");
        }
        if let Err(error) = self.driver.close().await {
            failures.push(format!("关闭 HmDriver 失败：{}", error_chain(&error)));
        }
        if failures.is_empty() {
            Ok(())
        } else {
            bail!(failures.join("；"))
        }
    }

    /// 先停后启，让 AppGallery 从干净的前台状态开始。
    ///
    /// 停不掉只记 warning；启动失败或等待前台超时属于结构性失败。
    async fn start_appgallery(&mut self) -> Result<()> {
        info!(device = %self.device_label, "正在关闭 AppGallery");
        if let Err(error) = self.driver.stop_app(&self.bundle).await {
            warn!(
                device = %self.device_label,
                error = %error_chain(&error),
                "关闭 AppGallery 时出现警告"
            );
        }
        sleep(APP_STOP_SETTLE).await;

        info!(device = %self.device_label, "正在启动 AppGallery");
        self.driver
            .start_app(&self.bundle, Some(APPGALLERY_ABILITY))
            .await
            .context("启动 AppGallery 失败")?;
        if !self.wait_for_appgallery(Duration::from_secs(15)).await? {
            bail!("等待 AppGallery 前台超时");
        }
        sleep(APP_START_SETTLE).await;
        info!(device = %self.device_label, "AppGallery 启动完成");
        Ok(())
    }

    /// 等待 AppGallery 进入前台；只有查询本身失败才返回错误，是否超时交给调用方判断。
    async fn wait_for_appgallery(&self, timeout: Duration) -> Result<bool> {
        self.driver
            .wait_for_app(&self.bundle, timeout)
            .await
            .context("查询 AppGallery 前台状态失败")
    }

    /// 单个页签的遍历：切到「应用」/「游戏」页签，进入分类入口，再遍历分类列表。
    async fn traverse_page(&mut self, page: &str) -> Result<()> {
        info!(device = %self.device_label, page, "开始遍历分类页面");
        self.click_app_or_game(page).await?;
        self.click_categories_tab().await?;
        self.pull_categories(page).await?;
        info!(device = %self.device_label, page, "分类页面遍历完成");
        Ok(())
    }

    /// 点击「应用」或「游戏」页签。
    ///
    /// 同时接受页签文本和底部图标 key，兼容不同版本 AppGallery 的页签结构。
    async fn click_app_or_game(&self, page: &str) -> Result<()> {
        let key = match page {
            "应用" => Some("BadgeImage.sys.symbol.bag_fill"),
            "游戏" => Some("BadgeImage.sys.symbol.game_fill"),
            _ => None,
        };
        self.click_local(
            move |node| {
                node.attribute_str("text") == Some(page)
                    || key.is_some_and(|key| node.attribute_str("key") == Some(key))
            },
            &format!("{page}页签"),
            Duration::from_secs(12),
        )
        .await?;
        sleep(PAGE_CLICK_SETTLE).await;
        Ok(())
    }

    /// 点击「分类」入口，并确认分类列表已经出现。
    async fn click_categories_tab(&self) -> Result<()> {
        self.click_local(
            |node| {
                let text = node.attribute_str("text");
                let key = node.attribute_str("key");
                text == Some("分类")
                    || matches!(
                        key,
                        Some(
                            "Paf_Lantern_Button_Index_1"
                                | "Paf_Lantern_Text_1"
                                | "Paf_Lantern_Normal_Image_1"
                                | "Paf_Lantern_Select_Image_1"
                        )
                    )
            },
            "分类入口",
            Duration::from_secs(12),
        )
        .await?;
        sleep(PAGE_CLICK_SETTLE).await;
        self.wait_for_categories(Duration::from_secs(12)).await?;
        Ok(())
    }

    /// 反复下滑分类列表，进入每个新发现的分类。
    ///
    /// 连续两轮没有新分类即认为已经到底；分类按钮为空属于结构性失败。
    async fn pull_categories(&mut self, page: &str) -> Result<()> {
        let mut seen = HashSet::new();
        let mut no_progress = 0usize;

        for _ in 0..MAX_CATEGORY_SCROLLS {
            let tree = self.wait_for_categories(Duration::from_secs(12)).await?;
            let buttons = category_buttons(&tree);
            if buttons.is_empty() {
                bail!("[{page}] 未找到分类按钮");
            }

            let mut discovered = false;
            for button in buttons {
                if !seen.insert(button.name.clone()) {
                    continue;
                }
                discovered = true;
                if self.config.skipped_categories.contains(&button.name) {
                    info!(device = %self.device_label, page, category = %button.name, "跳过分类");
                    continue;
                }
                self.collect_category(page, button).await?;
            }

            if discovered {
                no_progress = 0;
            } else {
                no_progress += 1;
            }
            if no_progress >= 2 {
                info!(device = %self.device_label, page, categories = seen.len(), "分类列表已遍历到底");
                return Ok(());
            }
            self.scroll_up().await?;
            sleep(CATEGORY_SCROLL_SETTLE).await;
        }

        bail!("[{page}] 分类列表超过 [{MAX_CATEGORY_SCROLLS}] 次仍未到底")
    }

    /// 进入一个分类：有子分类就遍历子分类，否则把当前页面直接当作应用列表。
    ///
    /// 分类页既没有子分类也没有应用卡片时只记 warning 跳过，最后无论走哪条分支都要
    /// 退回分类列表，保证下一个分类还能继续。
    async fn collect_category(&mut self, page: &str, button: CategoryButton) -> Result<()> {
        info!(
            device = %self.device_label,
            page,
            category = %button.name,
            "进入分类并遍历应用列表"
        );
        self.driver.click(button.bounds.center()).await?;
        sleep(self.config.category_click_settle).await;

        let content = self
            .wait_for_category_content(Duration::from_secs(12))
            .await?;
        if subcategory_buttons(&content).is_empty() {
            if app_snapshot(&content).is_empty() {
                // 偶发空帧或页面还在切换时跳过该分类，避免一台设备的偶发问题导致整轮失败。
                warn!(
                    device = %self.device_label,
                    page,
                    category = %button.name,
                    "分类页面无内容，跳过该分类"
                );
            } else {
                let count = self.drain_app_list(content, &button.name).await?;
                info!(device = %self.device_label, category = %button.name, apps = count, "分类应用列表遍历完成");
            }
        } else {
            self.drain_subcategories(&button.name).await?;
        }
        self.back_to_categories().await
    }

    /// 逐屏下滑子分类列表，进入每个子分类读取应用名称后返回分类页。
    ///
    /// 与分类列表相同：连续两轮没有新子分类即认为到底。
    async fn drain_subcategories(&mut self, category: &str) -> Result<()> {
        let mut seen = HashSet::new();
        let mut no_progress = 0usize;

        for _ in 0..MAX_CATEGORY_SCROLLS {
            let tree = self
                .wait_for_category_content(Duration::from_secs(12))
                .await?;
            let buttons = subcategory_buttons(&tree);
            let mut discovered = false;

            for button in buttons {
                if !seen.insert(button.name.clone()) {
                    continue;
                }
                discovered = true;
                info!(device = %self.device_label, category, subcategory = %button.name, "进入子分类");
                self.driver.click(button.bounds.center()).await?;
                sleep(self.config.category_click_settle).await;
                let content = self
                    .wait_for_app_list(Duration::from_secs(12), &button.name)
                    .await?;
                let count = self.drain_app_list(content, &button.name).await?;
                info!(device = %self.device_label, category, subcategory = %button.name, apps = count, "子分类应用列表遍历完成");
                self.driver.go_back().await?;
                sleep(BACK_SETTLE).await;
            }

            if discovered {
                no_progress = 0;
            } else {
                no_progress += 1;
            }
            if no_progress >= 2 {
                info!(device = %self.device_label, category, subcategories = seen.len(), "子分类列表已遍历到底");
                return Ok(());
            }
            self.scroll_up().await?;
            sleep(CATEGORY_SCROLL_SETTLE).await;
        }

        bail!("分类 [{category}] 的子分类超过 [{MAX_CATEGORY_SCROLLS}] 次仍未到底")
    }

    /// 把一个应用列表下滑到稳定，返回收集到的去重应用名数量。
    ///
    /// 空快照或名称不再增长都视为已经到底并正常返回；只有超过 MAX_SCROLLS 次仍在下滑
    /// 才报错。
    async fn drain_app_list(&self, mut tree: UiNode, category: &str) -> Result<usize> {
        let mut seen_names = HashSet::new();

        for _ in 0..MAX_SCROLLS {
            let snapshot = app_snapshot(&tree);
            if snapshot.is_empty() {
                // 偶发空帧不应该让整台设备失败，直接结束本轮读取。
                warn!(
                    device = %self.device_label,
                    category,
                    "当前页面未找到应用卡片，结束该列表读取"
                );
                return Ok(seen_names.len());
            }
            let before = seen_names.len();
            seen_names.extend(snapshot.into_iter().map(|entry| entry.name));
            if before == seen_names.len() {
                return Ok(seen_names.len());
            }

            // 每轮下滑两次，保持已有的遍历节奏。
            for _ in 0..2 {
                self.scroll_up().await?;
                sleep(LIST_SCROLL_SETTLE).await;
            }
            tree = self
                .wait_for_app_list(Duration::from_secs(12), category)
                .await?;
        }

        bail!("分类 [{category}] 的应用列表超过 [{MAX_SCROLLS}] 次仍未到底")
    }

    /// 返回分类列表页，并确认分类按钮已经重新出现。
    async fn back_to_categories(&self) -> Result<()> {
        self.driver.go_back().await?;
        sleep(BACK_SETTLE).await;
        self.wait_for_categories(Duration::from_secs(15)).await?;
        Ok(())
    }

    /// 等待分类列表出现；这里作为结构性判据，超时直接报错。
    async fn wait_for_categories(&self, timeout: Duration) -> Result<UiNode> {
        self.driver
            .wait_for_ui_tree(timeout, |tree| !category_buttons(tree).is_empty())
            .await
            .context("等待分类列表超时")
    }

    /// 等待分类页内容；超时只记录警告并退化为当前 UI，由调用方按空内容跳过。
    async fn wait_for_category_content(&self, timeout: Duration) -> Result<UiNode> {
        match self
            .driver
            .wait_for_ui_tree(timeout, |tree| {
                !subcategory_buttons(tree).is_empty() || !app_snapshot(tree).is_empty()
            })
            .await
        {
            Ok(tree) => Ok(tree),
            Err(error) => {
                warn!(
                    device = %self.device_label,
                    error = %error_chain(&error),
                    "等待分类内容超时，按空内容继续"
                );
                self.driver.ui_tree().await.context("获取当前 UI 树失败")
            }
        }
    }

    /// 等待应用列表；超时只记录警告并退化为当前 UI，让空列表被调用方跳过。
    async fn wait_for_app_list(&self, timeout: Duration, category: &str) -> Result<UiNode> {
        match self
            .driver
            .wait_for_ui_tree(timeout, |tree| !app_snapshot(tree).is_empty())
            .await
        {
            Ok(tree) => Ok(tree),
            Err(error) => {
                warn!(
                    device = %self.device_label,
                    category,
                    error = %error_chain(&error),
                    "等待应用列表超时，按空列表继续"
                );
                self.driver
                    .ui_tree()
                    .await
                    .with_context(|| format!("分类 [{category}] 获取当前 UI 树失败"))
            }
        }
    }

    async fn scroll_up(&self) -> Result<()> {
        self.driver
            .swipe_direction(SwipeDirection::Up, SwipeArea::FullScreen, 0.7, 2_000)
            .await
            .context("滚动 UI 失败")
    }

    /// 等待谓词命中的控件出现，找到可点击目标后点击其中心。
    ///
    /// 控件本身没有有效点击目标或 bounds 时报错。
    async fn click_local<F>(&self, predicate: F, description: &str, timeout: Duration) -> Result<()>
    where
        F: Fn(&UiNode) -> bool,
    {
        let tree = self
            .driver
            .wait_for_ui_tree(timeout, |tree| tree.find(&predicate).is_some())
            .await
            .with_context(|| format!("等待 [{description}] 超时"))?;
        let node = tree
            .find_click_target(&predicate)
            .ok_or_else(|| anyhow!("控件 [{description}] 没有有效点击目标"))?;
        let bounds = node
            .bounds()
            .ok_or_else(|| anyhow!("控件 [{description}] 没有有效 bounds"))?;
        self.driver.click(bounds.center()).await?;
        Ok(())
    }
}

/// 从当前页面里查找固定的子分类入口。
///
/// 新版 AppGallery 的入口层级是「可点击 Column -> Row -> Text」，Row 和 Text 自身都
/// 不是 Button，所以这里只按文本找点击目标，不要求控件类型。
fn subcategory_buttons(tree: &UiNode) -> Vec<CategoryButton> {
    SUBCATEGORY_NAMES
        .iter()
        .filter_map(|&name| {
            let target = tree.find_click_target(|node| node.attribute_str("text") == Some(name))?;
            Some(CategoryButton {
                name: name.to_owned(),
                bounds: target.bounds()?,
            })
        })
        .collect()
}

/// 新分类页面上固定的子分类入口名称。
const SUBCATEGORY_NAMES: &[&str] = &["新鲜应用", "新鲜游戏", "时下畅销应用", "时下畅销游戏"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 能识别新界面的子分类() {
        assert!(SUBCATEGORY_NAMES.contains(&"新鲜应用"));
        assert!(SUBCATEGORY_NAMES.contains(&"时下畅销游戏"));
        assert!(!SUBCATEGORY_NAMES.contains(&"工具"));
    }

    #[test]
    fn 子分类入口不要求_button_控件() {
        // 实机新版 AppGallery 的入口层级为 clickable Column -> Row -> Text，
        // Row 和 Text 自身均不是 Button。
        let tree: UiNode = serde_json::from_value(serde_json::json!({
            "attributes": {"type": "Root"},
            "children": [{
                "attributes": {
                    "type": "Column", "clickable": "true",
                    "bounds": "[25,135][3095,190]"
                },
                "children": [{
                    "attributes": {
                        "type": "Row", "text": "新鲜应用",
                        "bounds": "[50,137][3005,190]"
                    },
                    "children": [{
                        "attributes": {
                            "type": "Text", "text": "新鲜应用",
                            "bounds": "[50,137][149,190]"
                        },
                        "children": []
                    }]
                }]
            }]
        }))
        .unwrap();

        let buttons = subcategory_buttons(&tree);
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0].name, "新鲜应用");
    }
}
