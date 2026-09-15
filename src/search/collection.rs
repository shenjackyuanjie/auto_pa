//! 分类与应用列表的滚动收集。
//!
//! `SearchFlow::collect_all_categories` 在「应用」页签（以及非 `random_mode` 时的「游戏」
//! 页签）下点开分类页，`pull_categories` 反复滚动分类列表并把每个没见过的分类交给
//! `collect_category`，分类里的应用名收齐后并入 `SearchState` 并立即落盘。
//!
//! 进入分类之后有三条深度不同的路径，由 `collect_category` 分发：
//!
//! - 默认：只读分类首页自己的应用列表，固定 3 屏（`collect_category_list`）；
//! - `--random`：点「新鲜应用」入口，只读那个子页面 3 屏（`collect_category_fresh`）；
//! - `--deep`：按 `hilog` 的深度，分类页出现固定子分类入口就逐个进去、列表滑到底
//!   （`collect_category_deep`）。
//!
//! 容错策略与 `hilog` 对齐：等待子分类或应用列表超时只记 warning 并退化为当前 UI，空页面
//! 结束当前层级的读取；只有找不到分类按钮、超过滚动上限这类结构性异常才报错。降级可能
//! 漏掉名称，但首轮搜索后的刷新遍历会把两个页签重爬一遍。
//!
//! 分类列表和应用列表都必须滚到底才能收全，所以这里用稳定判据代替固定次数：分类列表连续
//! 两轮没有出现新按钮即认为到底（`no_progress` 计数），应用列表则比较前后两次
//! [`app_snapshot`] 是否完全一致。

use anyhow::{Context, Result, anyhow, bail};
use hm_driver_rs::UiNode;
use std::collections::HashSet;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::appgallery::{
    AppEntry, CategoryButton, app_snapshot, category_buttons, subcategory_buttons,
};
use crate::logging::error_chain;
use crate::search::flow::{
    BACK_SETTLE, CATEGORY_CLICK_SETTLE, CATEGORY_CONTENT_TIMEOUT, CATEGORY_SCROLL_SETTLE,
    MAX_CATEGORY_SCROLLS, SearchFlow,
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
    /// 「游戏」页签不在这一轮范围内。`deep_mode` 只影响进入分类后的爬取深度，不改变页签范围。
    pub(crate) async fn collect_all_categories(&mut self) -> Result<()> {
        self.start_appgallery().await?;
        // 新一轮爬取意味着可能又发现了一批应用，同开发者应用要重新收集一遍：清掉上一轮
        // 记下的开发者，避免刷新遍历之后仍按旧名单跳过。
        self.collected_developers.clear();

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
    /// 按爬取深度分发到三条路径（见模块文档），三条都自己负责退回分类列表页。
    async fn collect_category(&mut self, button: CategoryButton) -> Result<()> {
        info!(
            device = %self.device_label,
            category = %button.name,
            "进入应用分类"
        );
        self.driver.click(button.bounds.center()).await?;
        sleep(CATEGORY_CLICK_SETTLE).await;

        if self.deep_mode {
            self.collect_category_deep(&button.name).await
        } else if self.random_mode {
            self.collect_category_fresh(&button.name).await
        } else {
            self.collect_category_list(&button.name).await
        }
    }

    /// 默认路径：把分类首页自己的应用列表读 3 屏，然后退回分类列表。
    async fn collect_category_list(&mut self, category: &str) -> Result<()> {
        let current = self.wait_for_category_app_list(category).await?;
        let names = self.collect_category_app_list(current, category).await?;
        self.back_to_categories(1).await?;
        self.save_collected_names(category, names)
    }

    /// `--random` 路径：点开「新鲜应用」子页面，只读那个页面的 3 屏。
    ///
    /// 等不到入口说明这个分类没有「新鲜应用」区块，记 warning 跳过；跳过后同样要退回分类
    /// 列表，保证下一个分类还能继续。
    async fn collect_category_fresh(&mut self, category: &str) -> Result<()> {
        let Some(initial) = self.wait_for_fresh_apps(category).await? else {
            return self.back_to_categories(1).await;
        };
        self.click_fresh_apps(initial).await?;
        let app_layout = self.wait_for_category_app_list(category).await?;
        let names = self.collect_category_app_list(app_layout, category).await?;
        self.back_to_categories(2).await?;
        self.save_collected_names(category, names)
    }

    /// `--deep` 路径：按 `hilog` 的深度遍历这个分类。
    ///
    /// 分类页出现固定子分类入口时逐个进入、每个列表滑到底；没有子分类就把分类首页自己
    /// 当作应用列表滑到底。两者都只多退一级就能回到分类列表。
    async fn collect_category_deep(&mut self, category: &str) -> Result<()> {
        let content = self.wait_for_category_content(category).await?;
        if !subcategory_buttons(&content).is_empty() {
            self.drain_subcategories(category).await?;
            return self.back_to_categories(1).await;
        }
        if app_snapshot(&content).is_empty() {
            // 偶发空帧或页面还在切换时跳过该分类，与 hilog 的容错一致。
            warn!(
                device = %self.device_label,
                category,
                "分类页面无内容，跳过该分类"
            );
            return self.back_to_categories(1).await;
        }
        let names = self.collect_app_list(content, true).await?;
        self.back_to_categories(1).await?;
        self.save_collected_names(category, names)
    }

    /// 逐个进入分类页的子分类读取应用名，读完退回分类页。
    ///
    /// 收工判据与分类列表相同：连续两轮没有出现新子分类才认为到底；每个子分类的列表都由
    /// [`SearchFlow::collect_app_list`] 滑到底。
    async fn drain_subcategories(&mut self, category: &str) -> Result<()> {
        let mut seen = HashSet::new();
        let mut no_progress = 0usize;

        for _ in 0..MAX_CATEGORY_SCROLLS {
            let tree = self.wait_for_category_content(category).await?;
            let mut discovered = false;

            for button in subcategory_buttons(&tree) {
                if !seen.insert(button.name.clone()) {
                    continue;
                }
                discovered = true;
                info!(
                    device = %self.device_label,
                    category,
                    subcategory = %button.name,
                    "进入子分类"
                );
                self.driver.click(button.bounds.center()).await?;
                sleep(CATEGORY_CLICK_SETTLE).await;
                let app_layout = self.wait_for_category_app_list(&button.name).await?;
                let names = self.collect_app_list(app_layout, true).await?;
                self.save_collected_names(&button.name, names)?;
                self.driver.go_back().await?;
                sleep(BACK_SETTLE).await;
            }

            if discovered {
                no_progress = 0;
            } else {
                no_progress += 1;
            }
            if no_progress >= 2 {
                info!(
                    device = %self.device_label,
                    category,
                    subcategories = seen.len(),
                    "子分类遍历完成"
                );
                return Ok(());
            }
            self.scroll_up().await?;
            sleep(CATEGORY_SCROLL_SETTLE).await;
        }

        bail!("分类 [{category}] 的子分类超过 [{MAX_CATEGORY_SCROLLS}] 次仍未到底")
    }

    /// 等待分类页出现「新鲜应用」入口；超时只记 warning 并返回 `None`，由调用方跳过该分类。
    async fn wait_for_fresh_apps(&self, category: &str) -> Result<Option<UiNode>> {
        match self
            .driver
            .wait_for_ui_tree(CATEGORY_CONTENT_TIMEOUT, |tree| {
                tree.find(|node| node.attribute_str("text") == Some(FRESH_APPS_TEXT))
                    .is_some()
            })
            .await
        {
            Ok(tree) => Ok(Some(tree)),
            Err(error) => {
                warn!(
                    device = %self.device_label,
                    category,
                    entry = FRESH_APPS_TEXT,
                    error = %error_chain(&error),
                    "等待新鲜应用入口超时，跳过该分类"
                );
                Ok(None)
            }
        }
    }

    /// 等待分类页内容：出现固定子分类入口或应用卡片都算有内容。
    ///
    /// 超时只记 warning 并退化为当前 UI，由调用方按空内容跳过——与 `hilog` 的
    /// `wait_for_category_content` 一致。
    async fn wait_for_category_content(&self, category: &str) -> Result<UiNode> {
        match self
            .driver
            .wait_for_ui_tree(CATEGORY_CONTENT_TIMEOUT, |tree| {
                !subcategory_buttons(tree).is_empty() || !app_snapshot(tree).is_empty()
            })
            .await
        {
            Ok(tree) => Ok(tree),
            Err(error) => {
                warn!(
                    device = %self.device_label,
                    category,
                    error = %error_chain(&error),
                    "等待分类内容超时，按空内容继续"
                );
                self.driver
                    .ui_tree()
                    .await
                    .with_context(|| format!("分类 [{category}] 获取当前 UI 树失败"))
            }
        }
    }

    /// 等待分类页的应用卡片出现。
    ///
    /// 超时只记 warning 并退化为当前 UI，由调用方按空列表结束这一层级的读取；只有取树本身
    /// 失败才报错——与 `hilog` 的 `wait_for_app_list` 一致。
    async fn wait_for_category_app_list(&self, category: &str) -> Result<UiNode> {
        debug!(
            device = %self.device_label,
            category,
            timeout = ?CATEGORY_CONTENT_TIMEOUT,
            "等待分类应用列表"
        );
        match self
            .driver
            .wait_for_ui_tree(CATEGORY_CONTENT_TIMEOUT, |tree| {
                !app_snapshot(tree).is_empty()
            })
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
    /// 列表只记 warning 并返回已收集的名称（`--deep` 用），否则视为错误（搜索结果页与
    /// 开发者列表页用）。返回时列表仍停在底部，调用方若要点击靠前的结果得先滚回顶部。
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
                    // `--deep` 的列表可能因为页面还在切换而读到空帧：只记 warning 并结束
                    // 这次读取，与 hilog 的 `drain_app_list` 一致。
                    warn!(
                        device = %self.device_label,
                        total = names.len(),
                        "当前页面未找到应用卡片，结束该列表读取"
                    );
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

    /// 收集一个应用列表的固定 3 屏（`page` 为 `0..=2`）。
    ///
    /// 默认路径与 `--random` 路径共用它：进的是分类首页或「新鲜应用」子页面。它不做前后
    /// 快照比较，而是每滚一屏就重新等待应用列表出现再读一份快照，名称按 `seen_names` 去重；
    /// 某一屏读不到应用卡片时只记 warning 并返回已读到的名称，与 `hilog` 的容错一致。
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
                warn!(
                    device = %self.device_label,
                    category,
                    page = page + 1,
                    collected = names.len(),
                    "当前页面未找到应用卡片，结束该分类读取"
                );
                return Ok(names);
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
