//! 打开搜索结果里的第一个应用，收集「同开发者的应用」后原路返回。
//!
//! 平板上的应用详情页向下滚动会出现「同开发者的应用」区块，点区块右上角的「更多」
//! 入口会进入同名列表页，列表页把该开发者的全部应用按三列网格纵向排列。整个流程
//! 结束后必须回到搜索结果页，后续的搜索结果收集与「返回」才能继续。
//!
//! 前置页面状态：`execution::search_once` 刚进入搜索结果页，并把它当时的第一屏布局
//! 作为 `result_page` 传进来，详情页流程结束后由调用方在该页面上继续收集应用列表。
//! 因此这里既不负责打开搜索页，也不负责离开搜索结果页；异常中断时用
//! `recover_to_result_page` 把界面拉回搜索结果页，避免后续步骤在未知页面上继续操作。

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
/// 详情页向下查找区块的最大滚动次数。
const DETAIL_SCROLL_MAX: usize = 14;
const DETAIL_PAGE_TIMEOUT: Duration = Duration::from_secs(15);
const DEVELOPER_LIST_TIMEOUT: Duration = Duration::from_secs(12);
const RESULT_PAGE_TIMEOUT: Duration = Duration::from_secs(12);
const RECOVER_ATTEMPTS: usize = 3;

impl SearchFlow {
    /// 打开搜索结果里的第一个应用，收集它开发者的全部应用名并计入待搜索名称。
    ///
    /// 返回收集到的名称数量；详情页没有「同开发者的应用」区块时返回 0。无论成功
    /// 还是失败，调用方都应保证流程结束后停留在搜索结果页。
    ///
    /// 步骤：点第一个应用卡片进入详情页、向下滚动找到「同开发者的应用」区块、
    /// 点该区块标题行右侧的「更多」进入开发者应用列表页、读取列表里的名称并写入状态、
    /// 再连点两次返回（列表页 -> 详情页 -> 搜索结果页）。`result_page` 是刚进入结果页时的
    /// 布局快照，因此其中第一个应用卡片一定可见。
    ///
    /// 没有该区块的应用会直接返回 0，此时搜索结果页的返回按钮仍在，调用方可以继续
    /// 收集结果列表；其余失败由调用方用 `recover_to_result_page` 恢复页面。
    pub(crate) async fn collect_developer_apps(&mut self, result_page: &UiNode) -> Result<usize> {
        let entry = first_result_entry(result_page)
            .ok_or_else(|| anyhow!("搜索结果页没有应用卡片"))?;
        info!(app = %entry.name, "打开搜索结果里的第一个应用");
        self.driver.click(entry.bounds.center()).await?;
        self.wait_detail_page().await?;

        if !self.reveal_developer_section().await? {
            debug!(app = %entry.name, "详情页没有同开发者区块");
            self.back_to_result_page().await?;
            return Ok(0);
        }

        // 滚动后重新取树，保证「更多」入口的位置来自当前布局。
        let detail_page = self.driver.ui_tree().await?;
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

        self.back_to_result_page().await?;
        Ok(names.len())
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

        let bounds = target.ok_or_else(|| anyhow!("未找到 [{DEVELOPER_SECTION_TITLE}] 的更多入口"))?;
        debug!(
            left = bounds.left,
            top = bounds.top,
            "点击同开发者区块的更多入口"
        );
        self.driver.click(bounds.center()).await?;
        sleep(PAGE_CLICK_SETTLE).await;
        Ok(())
    }

    /// 开发者列表页 -> 详情页 -> 搜索结果页。
    ///
    /// 调用方只在进入开发者列表页之前失败时才会回到这里；此时后退两次会先回到搜索结果页，
    /// 结果页通常忽略多余的后退，最后仍由 `wait_result_page` 确认位置。
    async fn back_to_result_page(&self) -> Result<()> {
        for step in 0..2 {
            self.driver.go_back().await?;
            sleep(BACK_SETTLE).await;
            debug!(step, "从开发者应用流程返回上一级");
        }
        self.wait_result_page().await
    }

    /// 详情页流程意外中断时，把界面恢复到搜索结果页。
    ///
    /// 中断可能发生在详情页、开发者列表页或某个还没加载完的中间页，不知道需要后退几次，
    /// 因此每轮先看当前树里有没有搜索结果页的返回按钮：已经回到就立刻结束，否则后退一次
    /// 再判断，最多 `RECOVER_ATTEMPTS` 次，最后仍按搜索结果页继续等待并给出超时错误。
    pub(crate) async fn recover_to_result_page(&self) -> Result<()> {
        for attempt in 0..RECOVER_ATTEMPTS {
            let tree = self.driver.ui_tree().await?;
            if has_key(&tree, SEARCH_RESULT_BACK_KEY) {
                return Ok(());
            }
            debug!(attempt, "尝试恢复到搜索结果页");
            self.driver.go_back().await?;
            sleep(BACK_SETTLE).await;
        }
        self.wait_result_page().await
    }

    /// 以搜索结果页的返回按钮为准，确认已经离开详情页与开发者列表页。
    async fn wait_result_page(&self) -> Result<()> {
        self.driver
            .wait_for_ui(
                RESULT_PAGE_TIMEOUT,
                |node| node.attribute_str("key") == Some(SEARCH_RESULT_BACK_KEY),
            )
            .await
            .context("等待搜索结果页超时")?;
        Ok(())
    }
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
