//! 打开搜索结果里的第一个应用，收集「同开发者的应用」后原路返回。
//!
//! 平板上的应用详情页向下滚动会出现「同开发者的应用」区块，最多展示三行三列。区块没
//! 填满时它就是该开发者的全部应用，直接读区块即可；填满时点区块右上角的「更多」入口
//! 进入同名列表页，列表页把该开发者的全部应用按三列网格纵向排列。整个流程结束后必须
//! 回到搜索结果页，后续的搜索结果收集与「返回」才能继续。
//!
//! 同一个开发者往往有多个应用被搜到，而这套流程（下滑找区块 + 进列表页划到底）是整轮
//! 搜索里最贵的一步，因此详情页一打开就先读头部的开发者名（见 [`detail_developer_name`]）：
//! 本轮已经收过这个开发者就只退回搜索结果页，不再重复收集；名称记在
//! [`SearchFlow::collected_developers`] 里，每轮分类遍历开始时清空。
//!
//! 前置页面状态：`execution::search_once` 刚进入搜索结果页，并把它当时的第一屏布局
//! 作为 `result_page` 传进来，详情页流程结束后由调用方在该页面上继续收集应用列表。
//! 因此这里既不负责打开搜索页，也不负责离开搜索结果页；异常中断时用
//! `back_to_result_page` 把界面拉回搜索结果页，避免后续步骤在未知页面上继续操作。

use anyhow::{Context, Result, anyhow};
use hm_driver_rs::{Bounds, SwipeArea, SwipeDirection, UiNode};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::appgallery::{AppEntry, app_snapshot};
use crate::search::execution::SEARCH_RESULT_BACK_KEY;
use crate::search::flow::{BACK_SETTLE, DETAIL_SCROLL_SETTLE, PAGE_CLICK_SETTLE, SearchFlow};

/// 详情页里的区块标题，同时也是开发者应用列表页的页面标题。
const DEVELOPER_SECTION_TITLE: &str = "同开发者的应用";
/// 应用详情页根节点的 key，用来确认详情页已经打开。
const APP_DETAIL_PAGE_KEY: &str = "AppDetailPage";
/// 页面标题的 key，开发者列表页据此与详情页里的同名区块区分。
const PAGE_TITLE_KEY: &str = "__NavdestinationField__Text__MainTitle__";
/// 详情页头部「开发者」标签的文本。
const DEVELOPER_LABEL_TEXT: &str = "开发者";
/// 详情页头部「标签 + 值」里值节点的 key：安装量、年龄、分类、开发者名共用它。
const DETAIL_VALUE_KEY: &str = "detail_bottom_name";
/// 认定值节点与标签同列的最大横向偏差（像素）。
///
/// 头部四列的列距约 390px，同一列的值与标签中心 x 只差 0～2px，60px 足以容下渲染误差，
/// 又不会串到相邻列——串列会把「工具」这类分类值错当成开发者名。
const DEVELOPER_COLUMN_TOLERANCE: i32 = 60;
/// 详情页向下查找区块的最大滚动次数。
const DETAIL_SCROLL_MAX: usize = 14;
const DETAIL_PAGE_TIMEOUT: Duration = Duration::from_secs(15);
const DEVELOPER_LIST_TIMEOUT: Duration = Duration::from_secs(12);
const RESULT_PAGE_TIMEOUT: Duration = Duration::from_secs(12);
const RECOVER_ATTEMPTS: usize = 3;
/// 详情页区块一次最多展示三行三列；填满说明该开发者还有更多应用。
const DEVELOPER_PREVIEW_LIMIT: usize = 9;
/// 应用卡片名称节点在布局树里的 key。
const APP_NAME_KEY: &str = "app_name";

impl SearchFlow {
    /// 打开搜索结果里的第一个应用，收集它开发者的全部应用名并计入待搜索名称。
    ///
    /// 返回收集到的名称数量；详情页没有「同开发者的应用」区块时返回 0。无论成功
    /// 还是失败，调用方都应保证流程结束后停留在搜索结果页。
    ///
    /// 步骤：点第一个应用卡片进入详情页，先读头部的开发者名——本轮已经收过这个开发者就
    /// 直接退回结果页（返回 0），省掉后面整套动作。否则向下滚动找到「同开发者的应用」
    /// 区块：区块一次最多展示三行三列，没填满时它就是该开发者的全部应用，直接读取并用
    /// 一次后返回搜索结果页；填满或读不到卡片时才点标题行右侧的「更多」进入开发者应用
    /// 列表页，划到底读取全部名称，再逐级返回搜索结果页。`result_page` 是刚进入结果页时
    /// 的布局快照，因此其中第一个应用卡片一定可见。
    ///
    /// 没有该区块的应用会直接返回 0，此时搜索结果页的返回按钮仍在，调用方可以继续
    /// 收集结果列表；其余失败由调用方用 `back_to_result_page` 恢复页面。只有真正收完某个
    /// 开发者的应用才会把它记进 [`SearchFlow::collected_developers`]：失败时照旧留待下一个
    /// 同开发者的应用重试。
    pub(crate) async fn collect_developer_apps(&mut self, result_page: &UiNode) -> Result<usize> {
        let entry =
            first_result_entry(result_page).ok_or_else(|| anyhow!("搜索结果页没有应用卡片"))?;
        info!(app = %entry.name, "打开搜索结果里的第一个应用");
        self.driver.click(entry.bounds.center()).await?;
        self.wait_detail_page().await?;

        // 开发者名在详情页头部，不用滚动就能读到；这一步失败只是退化为不做去重。
        let developer = detail_developer_name(&self.driver.ui_tree().await?);
        debug!(
            app = %entry.name,
            developer = developer.as_deref().unwrap_or("<未读到>"),
            "详情页头部的开发者"
        );
        if let Some(developer) = developer.as_deref()
            && self.collected_developers.contains(developer)
        {
            info!(
                app = %entry.name,
                developer = %developer,
                "该开发者的应用本轮已收集过，跳过"
            );
            self.back_to_result_page().await?;
            return Ok(0);
        }

        if !self.reveal_developer_section().await? {
            debug!(app = %entry.name, "详情页没有同开发者区块");
            self.back_to_result_page().await?;
            return Ok(0);
        }

        // 滚动后重新取树，让区块内容与「更多」入口的位置都来自当前布局。
        let detail_page = self.driver.ui_tree().await?;
        let preview = developer_section_apps(&detail_page);
        if !preview.is_empty() && preview.len() < DEVELOPER_PREVIEW_LIMIT {
            let added = self.state.add_apps(preview.iter().cloned());
            self.store.save(&self.state)?;
            info!(
                app = %entry.name,
                collected = preview.len(),
                added,
                total = self.state.app_count(),
                "同开发者的应用不多，直接用详情页展示的名称"
            );
            self.remember_developer(developer);
            self.back_to_result_page().await?;
            return Ok(preview.len());
        }

        self.click_developer_more(&detail_page).await?;
        let list_page = self
            .driver
            .wait_for_ui_tree(DEVELOPER_LIST_TIMEOUT, is_developer_list_page)
            .await
            .context("等待同开发者应用列表超时")?;

        let names = self.collect_app_list(list_page, false).await?;
        let added = self.state.add_apps(names.iter().cloned());
        self.store.save(&self.state)?;
        info!(
            app = %entry.name,
            collected = names.len(),
            added,
            total = self.state.app_count(),
            "同开发者的应用收集完成"
        );
        self.remember_developer(developer);

        self.back_to_result_page().await?;
        Ok(names.len())
    }

    /// 记住一个刚刚收完的开发者，供本轮的其它应用跳过；读不到开发者名时什么也不做。
    fn remember_developer(&mut self, developer: Option<String>) {
        if let Some(developer) = developer {
            self.collected_developers.insert(developer);
        }
    }

    /// 等详情页根节点出现，避免详情页还没打开就去找区块。
    async fn wait_detail_page(&self) -> Result<()> {
        self.driver
            .wait_for_ui_tree(DETAIL_PAGE_TIMEOUT, |tree| {
                tree.find(|node| node.attribute_str("key") == Some(APP_DETAIL_PAGE_KEY))
                    .is_some()
            })
            .await
            .context("等待应用详情页超时")?;
        Ok(())
    }

    /// 逐屏向下滚动，直到出现「同开发者的应用」区块或详情页已经到底。
    ///
    /// 用整页文本签名判断是否还有进展：连续两屏签名相同说明已经到底，避免为了一个
    /// 不存在的区块白滚 `DETAIL_SCROLL_MAX` 次；滚到上限仍没出现也按没有该区块处理，
    /// 只有真的滚动失败才报错。
    async fn reveal_developer_section(&self) -> Result<bool> {
        let mut previous: Option<String> = None;
        for step in 0..DETAIL_SCROLL_MAX {
            let tree = self.driver.ui_tree().await?;
            if has_text(&tree, DEVELOPER_SECTION_TITLE) {
                debug!(step, "详情页出现同开发者区块");
                return Ok(true);
            }
            let signature = page_signature(&tree);
            if previous.as_deref() == Some(signature.as_str()) {
                debug!(step, "详情页已到底且没有同开发者区块");
                return Ok(false);
            }
            previous = Some(signature);
            self.driver
                .swipe_direction(SwipeDirection::Up, SwipeArea::FullScreen, 0.7, 2_000)
                .await
                .context("详情页滚动失败")?;
            sleep(DETAIL_SCROLL_SETTLE).await;
        }
        warn!(
            steps = DETAIL_SCROLL_MAX,
            "详情页滚动到上限仍未出现同开发者区块，按没有该区块处理"
        );
        Ok(false)
    }

    /// 点击区块标题行最右侧的「更多」入口，进入开发者应用列表页。
    ///
    /// 该入口没有 key 也没有 text，只能按「可点击、与标题同一行、位于标题右侧」定位。
    /// 同时满足条件的控件里取 `left` 最小者：那是标题右侧最靠近标题的一个，再往右才是
    /// 页面上的其它可点击控件。定位失败直接报错，交由调用方恢复页面。
    async fn click_developer_more(&self, tree: &UiNode) -> Result<()> {
        let title = tree
            .find(|node| node.attribute_str("text") == Some(DEVELOPER_SECTION_TITLE))
            .ok_or_else(|| anyhow!("未找到 [{DEVELOPER_SECTION_TITLE}] 标题"))?;
        let title_bounds = title
            .bounds()
            .ok_or_else(|| anyhow!("[{DEVELOPER_SECTION_TITLE}] 标题没有 bounds"))?;
        let row_y = title_bounds.center().y;

        let mut target: Option<Bounds> = None;
        for node in tree.find_all(|node| node.attribute_str("clickable") == Some("true")) {
            let Some(bounds) = node.bounds() else {
                continue;
            };
            if bounds.left < title_bounds.right || bounds.top > row_y || bounds.bottom < row_y {
                continue;
            }
            target = match target {
                Some(current) if current.left >= bounds.left => Some(current),
                _ => Some(bounds),
            };
        }

        let bounds =
            target.ok_or_else(|| anyhow!("未找到 [{DEVELOPER_SECTION_TITLE}] 的更多入口"))?;
        debug!(
            left = bounds.left,
            top = bounds.top,
            "点击同开发者区块的更多入口"
        );
        self.driver.click(bounds.center()).await?;
        sleep(PAGE_CLICK_SETTLE).await;
        Ok(())
    }

    /// 逐级后退，直到搜索结果页重新出现；正常收尾与意外中断都走这里。
    ///
    /// 调用点可能在详情页（没有「同开发者的应用」区块）或开发者列表页，两者需要后退的
    /// 层数不同，所以不能按固定次数后退：每轮先看当前布局树里有没有搜索结果页的返回
    /// 按钮，已经回到就结束，否则后退一次再判断，最多 `RECOVER_ATTEMPTS` 次；仍未回到
    /// 时按搜索结果页继续等待，好让超时错误指出真实原因。
    pub(crate) async fn back_to_result_page(&self) -> Result<()> {
        for attempt in 0..RECOVER_ATTEMPTS {
            let tree = self.driver.ui_tree().await?;
            if has_key(&tree, SEARCH_RESULT_BACK_KEY) {
                debug!(attempt, "已回到搜索结果页");
                return Ok(());
            }
            debug!(attempt, "返回上一级");
            self.driver.go_back().await?;
            sleep(BACK_SETTLE).await;
        }
        self.wait_result_page().await
    }

    /// 以搜索结果页的返回按钮为准，确认已经离开详情页与开发者列表页。
    async fn wait_result_page(&self) -> Result<()> {
        self.driver
            .wait_for_ui(RESULT_PAGE_TIMEOUT, |node| {
                node.attribute_str("key") == Some(SEARCH_RESULT_BACK_KEY)
            })
            .await
            .context("等待搜索结果页超时")?;
        Ok(())
    }
}

/// 读取详情页头部的开发者名。
///
/// 头部是一排「标签 + 值」：安装量 / 年龄 / 分类 / 开发者，值节点都带 `detail_bottom_name`
/// key，标签只有文本。这里先找文本为「开发者」的标签，再在同一列（中心 x 相差不超过
/// [`DEVELOPER_COLUMN_TOLERANCE`]）挑离标签最近的那个值节点。
///
/// 实机（平板、PC 布局）样例：标签 `开发者` 在 `[2445,277][2504,299]`，值是
/// `detail_bottom_name` 的 `[2358,344][2591,366]`；同一行的「次」「岁」「工具」三个值分别
/// 位于左边三列，中心 x 相差约 390px。
///
/// 读不到（页面没有该标签、值节点缺失或都不在同一列）时返回 `None`，调用方按「不知道
/// 开发者」处理，照旧走完整的同开发者收集流程——去重只是省时间，不能因此丢数据。
fn detail_developer_name(tree: &UiNode) -> Option<String> {
    let label = tree.find(|node| node.attribute_str("text") == Some(DEVELOPER_LABEL_TEXT))?;
    let label_bounds = label.bounds()?;
    let label_center_x = label_bounds.center().x;

    let mut best: Option<(i32, i32, String)> = None;
    for node in tree.find_all(|node| node.attribute_str("key") == Some(DETAIL_VALUE_KEY)) {
        let Some(bounds) = node.bounds() else {
            continue;
        };
        // 值在标签下方；同一行的其它列也会满足，靠中心 x 区分。
        if bounds.top < label_bounds.top {
            continue;
        }
        let Some(text) = node.attribute_str("text") else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let distance = (bounds.center().x - label_center_x).abs();
        if distance > DEVELOPER_COLUMN_TOLERANCE {
            continue;
        }
        // 横向距离相同时取更靠上的那个（开发者名换行时会拆成多个节点）。
        let candidate = (distance, bounds.top, text.to_owned());
        if best
            .as_ref()
            .is_none_or(|current| (candidate.0, candidate.1) < (current.0, current.1))
        {
            best = Some(candidate);
        }
    }
    best.map(|(_, _, name)| name)
}

/// 读取「同开发者的应用」区块里已经展示出来的应用名。
///
/// 区块一次最多渲染三行三列，所以返回数量小于 `DEVELOPER_PREVIEW_LIMIT` 时它就是这个
/// 开发者的全部应用，调用方不必再进列表页。页面上没有区块标题时返回空列表，交给调用方
/// 走列表页那条更保守的路径。
fn developer_section_apps(tree: &UiNode) -> Vec<String> {
    let Some(title) = tree.find(|node| node.attribute_str("text") == Some(DEVELOPER_SECTION_TITLE))
    else {
        return Vec::new();
    };
    let Some(title_bounds) = title.bounds() else {
        return Vec::new();
    };
    // 区块容器（ListItem）的底边就是下一块推荐的上边。
    let section_bottom =
        section_container(tree, title_bounds).map_or(i32::MAX, |bounds| bounds.bottom);

    let mut names = Vec::new();
    for node in tree.find_all(|node| node.attribute_str("key") == Some(APP_NAME_KEY)) {
        let Some(bounds) = node.bounds() else {
            continue;
        };
        if bounds.top < title_bounds.bottom || bounds.top >= section_bottom {
            continue;
        }
        let Some(text) = node.attribute_str("text") else {
            continue;
        };
        if !text.is_empty() && !names.iter().any(|name| name == text) {
            names.push(text.to_owned());
        }
    }
    names
}

/// 找包含区块标题的最小 `ListItem`，用它把区块和下面的其它推荐分开。
fn section_container(tree: &UiNode, title_bounds: Bounds) -> Option<Bounds> {
    let mut best: Option<Bounds> = None;
    for node in tree.find_all(|node| node.attribute_str("type") == Some("ListItem")) {
        let Some(bounds) = node.bounds() else {
            continue;
        };
        if bounds.top > title_bounds.top || bounds.bottom < title_bounds.bottom {
            continue;
        }
        best = match best {
            Some(current) if bounds_area(current) <= bounds_area(bounds) => Some(current),
            _ => Some(bounds),
        };
    }
    best
}

fn bounds_area(bounds: Bounds) -> i32 {
    (bounds.right - bounds.left) * (bounds.bottom - bounds.top)
}

/// 结果页按先上后左的顺序取第一个应用卡片。
///
/// `app_snapshot` 的返回顺序取决于布局树遍历顺序，三列网格下不保证第一个就是左上角，
/// 因此显式按行再按列排序。
fn first_result_entry(tree: &UiNode) -> Option<AppEntry> {
    let mut entries = app_snapshot(tree);
    entries.sort_by_key(|entry| (entry.bounds.top, entry.bounds.left));
    entries.into_iter().next()
}

fn is_developer_list_page(tree: &UiNode) -> bool {
    tree.find(|node| {
        node.attribute_str("key") == Some(PAGE_TITLE_KEY)
            && node.attribute_str("text") == Some(DEVELOPER_SECTION_TITLE)
    })
    .is_some()
}

fn has_text(tree: &UiNode, text: &str) -> bool {
    tree.find(|node| node.attribute_str("text") == Some(text))
        .is_some()
}

fn has_key(tree: &UiNode, key: &str) -> bool {
    tree.find(|node| node.attribute_str("key") == Some(key))
        .is_some()
}

/// 用页面上去重后的全部文本作为签名，用来判断滚动是否还有进展。
///
/// 排序去掉顺序差异后拼接，因此截图内容相同的相邻两屏签名一致；`reveal_developer_section`
/// 靠它识别详情页已经到底。
fn page_signature(tree: &UiNode) -> String {
    let mut items: Vec<String> = Vec::new();
    for node in tree.find_all(|node| {
        node.attribute_str("text")
            .is_some_and(|text| !text.is_empty())
    }) {
        if let Some(text) = node.attribute_str("text") {
            items.push(text.to_owned());
        }
    }
    items.sort();
    items.dedup();
    items.join("|")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn 详情页头部能读出开发者名() {
        // 实机详情页头部：四个「标签 + 值」列，值共用 detail_bottom_name key。
        let tree: UiNode = serde_json::from_value(json!({
            "attributes": {"type": "Root"},
            "children": [
                {"attributes": {"text": "安装", "bounds": "[1270,277][1309,299]"}, "children": []},
                {"attributes": {"key": "detail_top_name", "text": "<1万", "bounds": "[1262,305][1317,338]"}, "children": []},
                {"attributes": {"key": "detail_bottom_name", "text": "次", "bounds": "[1280,344][1300,366]"}, "children": []},
                {"attributes": {"text": "年龄", "bounds": "[1665,277][1704,299]"}, "children": []},
                {"attributes": {"key": "detail_top_name", "text": "3+", "bounds": "[1670,305][1700,338]"}, "children": []},
                {"attributes": {"key": "detail_bottom_name", "text": "岁", "bounds": "[1675,344][1695,366]"}, "children": []},
                {"attributes": {"text": "分类", "bounds": "[2060,277][2099,299]"}, "children": []},
                {"attributes": {"key": "detail_bottom_name", "text": "工具", "bounds": "[2060,344][2099,366]"}, "children": []},
                {"attributes": {"text": "开发者", "bounds": "[2445,277][2504,299]"}, "children": []},
                {"attributes": {"key": "detail_bottom_name", "text": "合肥棋言教育科技有限公司", "bounds": "[2358,344][2591,366]"}, "children": []}
            ]
        }))
        .unwrap();

        assert_eq!(
            detail_developer_name(&tree).as_deref(),
            Some("合肥棋言教育科技有限公司")
        );
    }

    #[test]
    fn 详情页没有开发者标签时读不出名字() {
        let tree: UiNode = serde_json::from_value(json!({
            "attributes": {"type": "Root"},
            "children": [
                {"attributes": {"text": "分类", "bounds": "[2060,277][2099,299]"}, "children": []},
                {"attributes": {"key": "detail_bottom_name", "text": "工具", "bounds": "[2060,344][2099,366]"}, "children": []}
            ]
        }))
        .unwrap();

        assert_eq!(detail_developer_name(&tree), None);
    }

    #[test]
    fn 相邻列的值不会被当成开发者名() {
        // 标签存在但同列没有值：宁可返回 None（不做去重），也不能把「工具」当开发者名。
        let tree: UiNode = serde_json::from_value(json!({
            "attributes": {"type": "Root"},
            "children": [
                {"attributes": {"text": "分类", "bounds": "[2060,277][2099,299]"}, "children": []},
                {"attributes": {"key": "detail_bottom_name", "text": "工具", "bounds": "[2060,344][2099,366]"}, "children": []},
                {"attributes": {"text": "开发者", "bounds": "[2445,277][2504,299]"}, "children": []}
            ]
        }))
        .unwrap();

        assert_eq!(detail_developer_name(&tree), None);
    }

    #[test]
    fn 详情页区块只统计标题下方当前区块的应用() {
        let tree: UiNode = serde_json::from_value(json!({
            "attributes": {"type": "Root"},
            "children": [
                {"attributes": {"key": "app_name", "text": "标题上方", "bounds": "[100,900][200,930]"}, "children": []},
                {"attributes": {"type": "ListItem", "bounds": "[0,940][3120,1320]"}, "children": [
                    {"attributes": {"text": "同开发者的应用", "bounds": "[50,950][300,1000]"}, "children": []},
                    {"attributes": {"key": "app_name", "text": "第一个", "bounds": "[100,1020][200,1050]"}, "children": []},
                    {"attributes": {"key": "app_name", "text": "第二个", "bounds": "[1100,1020][1200,1050]"}, "children": []}
                ]},
                {"attributes": {"type": "ListItem", "bounds": "[0,1320][3120,1700]"}, "children": [
                    {"attributes": {"text": "同分类的热门应用", "bounds": "[50,1330][300,1380]"}, "children": []},
                    {"attributes": {"key": "app_name", "text": "别的区块", "bounds": "[100,1400][200,1430]"}, "children": []}
                ]}
            ]
        }))
        .unwrap();
        assert_eq!(developer_section_apps(&tree), vec!["第一个", "第二个"]);
    }

    #[test]
    fn 没有同开发者区块时返回空列表() {
        let tree: UiNode = serde_json::from_value(json!({
            "attributes": {"type": "Root"},
            "children": [
                {"attributes": {"key": "app_name", "text": "推荐", "bounds": "[100,400][200,430]"}, "children": []}
            ]
        }))
        .unwrap();
        assert!(developer_section_apps(&tree).is_empty());
    }

    #[test]
    fn 第一个搜索结果取最上最左的应用() {
        let tree: UiNode = serde_json::from_value(json!({
            "attributes": {"type": "Root"},
            "children": [{"attributes": {"type": "List"}, "children": [
                {"attributes": {"key": "app_name", "text": "第二行", "bounds": "[100,500][200,530]"}, "children": []},
                {"attributes": {"key": "app_name", "text": "第三行", "bounds": "[100,900][200,930]"}, "children": []},
                {"attributes": {"key": "app_name", "text": "第一行右", "bounds": "[1100,300][1200,330]"}, "children": []},
                {"attributes": {"key": "app_name", "text": "第一行左", "bounds": "[100,300][200,330]"}, "children": []}
            ]}]
        }))
        .unwrap();
        let entry = first_result_entry(&tree).unwrap();
        assert_eq!(entry.name, "第一行左");
    }

    #[test]
    fn 开发者列表页按标题识别() {
        let list_page: UiNode = serde_json::from_value(json!({
            "attributes": {"type": "Root"},
            "children": [{"attributes": {
                "key": "__NavdestinationField__Text__MainTitle__",
                "text": "同开发者的应用"
            }, "children": []}]
        }))
        .unwrap();
        assert!(is_developer_list_page(&list_page));

        // 详情页里的同名区块只是普通文本，没有页面标题 key。
        let detail_page: UiNode = serde_json::from_value(json!({
            "attributes": {"type": "Root"},
            "children": [{"attributes": {"text": "同开发者的应用"}, "children": []}]
        }))
        .unwrap();
        assert!(!is_developer_list_page(&detail_page));
        assert!(has_text(&detail_page, DEVELOPER_SECTION_TITLE));
        assert!(!has_key(&detail_page, SEARCH_RESULT_BACK_KEY));
    }

    #[test]
    fn 页面签名忽略顺序与重复() {
        let first: UiNode = serde_json::from_value(json!({
            "attributes": {"type": "Root"},
            "children": [
                {"attributes": {"text": "甲"}, "children": []},
                {"attributes": {"text": "乙"}, "children": []},
                {"attributes": {"text": "甲"}, "children": []}
            ]
        }))
        .unwrap();
        let second: UiNode = serde_json::from_value(json!({
            "attributes": {"type": "Root"},
            "children": [
                {"attributes": {"text": "乙"}, "children": []},
                {"attributes": {"text": "甲"}, "children": []}
            ]
        }))
        .unwrap();
        assert_eq!(page_signature(&first), page_signature(&second));

        let third: UiNode = serde_json::from_value(json!({
            "attributes": {"type": "Root"},
            "children": [
                {"attributes": {"text": "乙"}, "children": []},
                {"attributes": {"text": "丙"}, "children": []}
            ]
        }))
        .unwrap();
        assert_ne!(page_signature(&first), page_signature(&third));
    }
}
