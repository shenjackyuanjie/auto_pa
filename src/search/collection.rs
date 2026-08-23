use anyhow::{Context, Result, anyhow, bail};
use hm_driver_rs::UiNode;
use std::collections::HashSet;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info};

use crate::appgallery::{AppEntry, CategoryButton, app_snapshot, category_buttons};
use crate::search::flow::{
    CATEGORY_CLICK_SETTLE, CATEGORY_CONTENT_TIMEOUT, CATEGORY_SCROLL_SETTLE, MAX_CATEGORY_SCROLLS,
    SearchFlow,
};

const FRESH_APPS_TEXT: &str = "新鲜应用";
const MAX_SCROLLS: usize = 100;
const SCROLL_BATCH_SIZE: usize = 2;
const SCROLL_WAIT: Duration = Duration::from_millis(100);

impl SearchFlow {
    pub(crate) async fn collect_all_categories(&mut self) -> Result<()> {
        self.start_appgallery().await?;

        let mut pages = vec!["应用"];
        if !self.random_mode {
            pages.push("游戏");
        }

        for page in pages {
            info!(device = %self.device_label, page, "开始收集分类");
            self.click_app_or_game(page).await?;
            self.click_categories_tab().await?;
            self.pull_categories(page).await?;
            info!(device = %self.device_label, page, "页面分类收集完成");
        }
        Ok(())
    }

    async fn pull_categories(&mut self, page: &str) -> Result<()> {
        let mut seen = HashSet::new();
        let mut no_progress = 0;
        info!(device = %self.device_label, page, "开始遍历分类列表");

        for _ in 0..MAX_CATEGORY_SCROLLS {
            let tree = self.wait_for_categories(Duration::from_secs(12)).await?;
            let buttons = category_buttons(&tree);
            if buttons.is_empty() {
                bail!("[{page}] 未找到分类按钮");
            }

            let mut opened = false;
            for button in buttons {
                if !seen.insert(button.name.clone()) {
                    continue;
                }
                opened = true;
                debug!(
                    device = %self.device_label,
                    page,
                    category = %button.name,
                    "发现分类按钮"
                );
                info!(
                    device = %self.device_label,
                    page,
                    category = %button.name,
                    "正在收集分类"
                );
                self.collect_category(button).await?;
            }

            if opened {
                no_progress = 0;
            } else {
                no_progress += 1;
            }
            if no_progress >= 2 {
                info!(
                    device = %self.device_label,
                    page,
                    categories = seen.len(),
                    "分类列表遍历完成"
                );
                return Ok(());
            }
            self.scroll_up().await?;
            sleep(CATEGORY_SCROLL_SETTLE).await;
        }

        bail!("[{page}] 分类列表超过 [{MAX_CATEGORY_SCROLLS}] 次仍未到底")
    }

    async fn collect_category(&mut self, button: CategoryButton) -> Result<()> {
        info!(
            device = %self.device_label,
            category = %button.name,
            "进入应用分类"
        );
        self.driver.click(button.bounds.center()).await?;
        sleep(CATEGORY_CLICK_SETTLE).await;

        if self.random_mode {
            let initial = self
                .driver
                .wait_for_ui_tree(CATEGORY_CONTENT_TIMEOUT, |tree| {
                    tree.find(|node| node.attribute_str("text") == Some(FRESH_APPS_TEXT))
                        .is_some()
                })
                .await
                .with_context(|| {
                    format!(
                        "进入分类 [{}] 后等待 [{}] 入口超时",
                        button.name, FRESH_APPS_TEXT
                    )
                })?;
            self.click_fresh_apps(initial).await?;
            let app_layout = self
                .driver
                .wait_for_ui_tree(CATEGORY_CONTENT_TIMEOUT, |tree| {
                    !app_snapshot(tree).is_empty()
                })
                .await
                .context("进入新鲜应用页面后未找到应用列表")?;
            let names = self
                .collect_category_app_list(app_layout, &button.name)
                .await?;
            self.back_to_categories(2).await?;
            self.save_collected_names(&button.name, names)?;
        } else {
            let current = self.wait_for_category_app_list(&button.name).await?;
            let names = self
                .collect_category_app_list(current, &button.name)
                .await?;
            self.back_to_categories(1).await?;
            self.save_collected_names(&button.name, names)?;
        }
        Ok(())
    }

    async fn wait_for_category_app_list(&self, category: &str) -> Result<UiNode> {
        debug!(
            device = %self.device_label,
            category,
            timeout = ?CATEGORY_CONTENT_TIMEOUT,
            "等待分类应用列表"
        );
        self.driver
            .wait_for_ui_tree(CATEGORY_CONTENT_TIMEOUT, |tree| {
                !app_snapshot(tree).is_empty()
            })
            .await
            .with_context(|| format!("分类 [{}] 等待应用列表超时", category))
    }

    async fn click_fresh_apps(&self, initial: UiNode) -> Result<()> {
        info!(device = %self.device_label, "查找新鲜应用入口");
        let node = initial
            .find(|node| node.attribute_str("text") == Some(FRESH_APPS_TEXT))
            .ok_or_else(|| anyhow!("进入分类后未找到 [{}] 入口", FRESH_APPS_TEXT))?;
        let bounds = node
            .bounds()
            .ok_or_else(|| anyhow!("新鲜应用控件没有 bounds"))?;
        self.driver.click(bounds.center()).await?;
        sleep(CATEGORY_CLICK_SETTLE).await;
        Ok(())
    }

    fn save_collected_names(&mut self, category: &str, names: Vec<String>) -> Result<()> {
        let added = self.state.add_apps(names.iter().cloned());
        self.store.save(&self.state)?;
        info!(
            device = %self.device_label,
            category,
            collected = names.len(),
            added,
            total = self.state.app_count(),
            "分类收集完成"
        );
        Ok(())
    }

    pub(crate) async fn collect_app_list(
        &self,
        initial: UiNode,
        allow_empty: bool,
    ) -> Result<Vec<String>> {
        let mut names = Vec::new();
        let mut seen_names = HashSet::new();
        let mut previous: Option<Vec<AppEntry>> = None;
        let mut current = Some(initial);

        for _ in 0..MAX_SCROLLS {
            let tree = match current.take() {
                Some(tree) => tree,
                None => self.driver.ui_tree().await?,
            };
            let snapshot = app_snapshot(&tree);
            if snapshot.is_empty() {
                if allow_empty {
                    return Ok(names);
                }
                bail!("当前页面未找到应用卡片");
            }

            for entry in &snapshot {
                if seen_names.insert(entry.name.clone()) {
                    names.push(entry.name.clone());
                }
            }

            if previous.as_ref() == Some(&snapshot) {
                debug!(
                    device = %self.device_label,
                    visible = snapshot.len(),
                    total = names.len(),
                    "应用列表达到稳定状态"
                );
                return Ok(names);
            }
            previous = Some(snapshot);

            for _ in 0..SCROLL_BATCH_SIZE {
                self.scroll_up().await?;
                sleep(SCROLL_WAIT).await;
            }
        }

        bail!("应用列表超过 [{MAX_SCROLLS}] 次仍未到底")
    }

    async fn collect_category_app_list(
        &self,
        initial: UiNode,
        category: &str,
    ) -> Result<Vec<String>> {
        let mut names = Vec::new();
        let mut seen_names = HashSet::new();
        let mut tree = initial;

        for page in 0..=2 {
            let snapshot = app_snapshot(&tree);
            if snapshot.is_empty() {
                bail!("分类 [{}] 当前页面未找到应用卡片", category);
            }
            debug!(
                device = %self.device_label,
                category,
                page = page + 1,
                visible = snapshot.len(),
                "读取分类应用名称"
            );
            for entry in snapshot {
                if seen_names.insert(entry.name.clone()) {
                    names.push(entry.name);
                }
            }

            if page < 2 {
                self.scroll_up().await?;
                sleep(CATEGORY_SCROLL_SETTLE).await;
                tree = self.wait_for_category_app_list(category).await?;
            }
        }
        Ok(names)
    }
}
