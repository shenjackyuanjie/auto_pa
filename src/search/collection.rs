//! 分类与应用列表的滚动收集。
//!
//! `SearchFlow::collect_all_categories` 在「应用」页签（以及非 `random_mode` 时的「游戏」
//! 页签）下点开分类页，`pull_categories` 反复滚动分类列表并把每个没见过的分类交给
//! `collect_category`，分类里的应用名收齐后并入 `SearchState` 并立即落盘。
//!
//! 分类列表和应用列表都必须滚到底才能收全，所以这里用稳定判据代替固定次数：分类列表连续
//! 两轮没有出现新按钮即认为到底（`no_progress` 计数），应用列表则比较前后两次
//! [`app_snapshot`] 是否完全一致。到达 `MAX_CATEGORY_SCROLLS` / `MAX_SCROLLS` 上限仍未
//! 稳定时按结构性错误报出，而不是返回一份不完整的名称列表。

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

/// 分类里的「新鲜应用」入口文本，`--random` 模式下每个分类只收这一页。
const FRESH_APPS_TEXT: &str = "新鲜应用";
/// 应用列表滚动收集的硬上限；超过它说明页面始终不稳定，报错比死循环更可取。
const MAX_SCROLLS: usize = 100;
/// 每次稳定判定之后连续滚动的屏数：一次多滚几屏能更快到底，也降低单次滑动没生效就被
/// 当成「已稳定」的概率。
const SCROLL_BATCH_SIZE: usize = 2;
/// 批量滚动中两屏之间的短暂等待；是否到底最终由前后两次 `app_snapshot` 的比较决定，
/// 因此不需要像分类翻页那样等满一个 settle。
const SCROLL_WAIT: Duration = Duration::from_millis(100);

impl SearchFlow {
    /// 遍历页签下的分类，把发现的全部应用名并入进度。
    ///
    /// `random_mode` 下只走「应用」页签：该模式只收集每个分类的「新鲜应用」列表，
    /// 「游戏」页签不在这一轮范围内。
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

    /// 反复滚动分类列表，进入每个还没见过的分类。
    ///
    /// 每一轮都重新取整棵树再算一次分类按钮：滚动之后按钮的 bounds 会变，必须按当前快照
    /// 点击。某一轮只要进了新分类就把 `no_progress` 清零，一轮没有任何新分类才累加，连续
    /// 两轮都没有新分类才认定到底——单轮没有新分类也可能只是滚动刚结束、下一批按钮还没
    /// 渲染出来，多确认一轮可以避免提前收工。超过 `MAX_CATEGORY_SCROLLS` 轮仍未到底属于
    /// 结构性异常，直接报错。
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

    /// 进入一个分类，收集其中的应用名，然后回到分类列表页。
    ///
    /// `random_mode` 下先在分类首页等「新鲜应用」入口出现并点进去，收完要退两级
    /// （新鲜应用页 -> 分类首页 -> 分类列表）；默认模式直接收分类首页的列表，只退一级。
    /// 两条路径都先返回分类列表页，再统一把名称并入进度。
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

    /// 等待分类页的应用卡片出现；默认模式用它做进入分类后的首屏等待，`random_mode` 下
    /// 每滚一屏也用它，超时统一为 [`CATEGORY_CONTENT_TIMEOUT`]。
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

    /// 点击分类页里的「新鲜应用」入口。
    ///
    /// 复用等待阶段那份已经确认入口存在的快照，不重新 dump，避免入口此时已经移出可视区。
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

    /// 去重后把名称并入进度并落盘；`added` 是本分类新贡献的名称数。
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

    /// 把应用列表滚动到底并收集名称；搜索结果页与开发者列表页也复用它。
    ///
    /// 结束判据是前后两次 [`app_snapshot`] 完全相等：`app_snapshot` 取应用卡片最多的那个
    /// `List`，只要还有新卡片进入可视区，快照就会变化，因此两份相同的快照意味着已经到底。
    /// 每次判定之后连滚 `SCROLL_BATCH_SIZE` 屏再取下一份快照，到底时最多多滚两屏，代价很小。
    ///
    /// `initial` 是调用方已经拿到的首屏快照，省掉一次 `ui_tree`；`allow_empty` 为真时空
    /// 列表直接返回已收集的名称，否则视为错误。返回时列表仍停在底部，调用方若要点击靠前
    /// 的结果得先滚回顶部。
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

    /// 收集分类首页的应用名，只读固定 3 屏（`page` 为 `0..=2`）。
    ///
    /// 这条路径只被 `random_mode` 使用，进的是「新鲜应用」子页面；它不做前后快照比较，
    /// 而是每滚一屏就重新等待应用列表出现再读一份快照，名称同样按 `seen_names` 去重。
    /// 任一屏读不到应用卡片就直接报错并向上传播，最终中止该设备的这一轮流程。
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
