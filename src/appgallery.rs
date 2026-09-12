//! AppGallery 的包名常量，以及从 uitest 布局树里识别页面元素的纯函数。
//!
//! 这里只解析 `UiNode`、不访问设备，因此可以直接用布局 JSON 做单元测试。分类收集
//! （`search::collection`）、搜索执行（`search::execution`）、开发者应用收集
//! （`search::developer`）和 `hilog` 的界面遍历都复用这两个选取函数来判断当前页面
//! 是分类列表还是应用列表。
//!
//! 平板（PC 布局）的一屏布局里往往有多个 `List`，导航栏和遮罩层也会被扫到，所以
//! `category_buttons` 与 `app_snapshot` 都按「有效元素最多」来挑列表，而不是取第一个。

use hm_driver_rs::{Bounds, UiNode};
use std::collections::HashSet;

/// AppGallery 的包名，启动、关闭应用与查询前台状态都用它。
pub const APPGALLERY_BUNDLE: &str = "com.huawei.hmsapp.appgallery";
/// AppGallery 的入口 Ability，启动应用时显式指定。
pub const APPGALLERY_ABILITY: &str = "MainAbility";

/// 一张应用卡片：卡片上显示的名称与它在屏幕上的位置。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppEntry {
    /// 卡片上显示的应用名。
    pub name: String,
    /// 卡片区域，点击时取其中点。
    pub bounds: Bounds,
}

/// 分类列表里的一个分类按钮。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategoryButton {
    /// 分类名称，同时用于日志和跨轮次去重。
    pub name: String,
    /// 按钮区域，点击时取其中点。
    pub bounds: Bounds,
}

/// 从所有 List 控件中选择最像分类列表的那个，并排除页面导航和操作按钮。
///
/// 选取策略：逐个 `List` 统计其中可用的按钮，取去重名称最多的那个。
///
/// 排除规则（与 `is_category_tab_or_action` 配合）：
/// - 文本命中 `is_category_tab_or_action` 的按钮，如底部导航标签和「安装/打开/更新」等操作；
/// - 同一个 `List` 内名称重复的按钮，只保留首次出现的那条；
/// - 没有非空文本子节点的按钮，以及按钮和文字节点都没有 `bounds` 的条目。
///
/// 返回的按钮按在布局树中出现的顺序排列；一个可用按钮都没有时返回空列表。
pub fn category_buttons(tree: &UiNode) -> Vec<CategoryButton> {
    let mut best = Vec::new();
    let mut best_score = 0usize;
    let list_nodes = tree.find_all(|node| node.attribute_str("type") == Some("List"));

    for list in list_nodes {
        let mut buttons = Vec::new();
        let mut unique_names = HashSet::new();
        for button in list.find_all(|node| node.attribute_str("type") == Some("Button")) {
            // 分类按钮自身没有 text，分类名挂在它的子 Text 节点上。
            let Some(text_node) = button.find(|node| {
                node.attribute_str("text")
                    .is_some_and(|text| !text.is_empty())
            }) else {
                continue;
            };
            let Some(name) = text_node.attribute_str("text") else {
                continue;
            };
            if is_category_tab_or_action(name) || !unique_names.insert(name.to_owned()) {
                continue;
            }
            // 点击目标是按钮本身；按钮没有 bounds 时退回文字节点。
            let Some(bounds) = button.bounds().or_else(|| text_node.bounds()) else {
                continue;
            };
            buttons.push(CategoryButton {
                name: name.to_owned(),
                bounds,
            });
        }
        // 用 >= 而非 >：有效按钮数量并列时取靠后的 List。
        if unique_names.len() >= best_score {
            best_score = unique_names.len();
            best = buttons;
        }
    }
    best
}

/// 选取应用卡片最多的 List，避免把导航栏或遮罩层误认为应用列表。
///
/// 所有 `List` 都没收集到卡片时，退回在整棵树上收集。
pub fn app_snapshot(tree: &UiNode) -> Vec<AppEntry> {
    let list_nodes = tree.find_all(|node| node.attribute_str("type") == Some("List"));
    let mut best = Vec::new();
    for list in list_nodes {
        let current = app_entries(list);
        if current.len() > best.len() {
            best = current;
        }
    }
    if best.is_empty() {
        app_entries(tree)
    } else {
        best
    }
}

/// 判断文本是否属于底部导航标签或卡片上的操作按钮，而不是分类名。
///
/// 这些文本会出现在同一个 `List` 里，若混进分类结果，遍历时就会去点错控件。
fn is_category_tab_or_action(value: &str) -> bool {
    matches!(
        value,
        "精选" | "分类" | "排行榜" | "重磅更新" | "安装" | "打开" | "更新"
    )
}

/// 在给定子树里按 `key == "app_name"` 收集应用卡片。
///
/// 跳过文本为空或没有 `bounds` 的节点；只有名称与位置完全相同的条目才会被去掉，
/// 同名但位置不同的卡片各保留一条，由调用方按名称去重。
fn app_entries(root: &UiNode) -> Vec<AppEntry> {
    let mut result = Vec::new();
    for node in root.find_all(|node| node.attribute_str("key") == Some("app_name")) {
        let Some(name) = node.attribute_str("text") else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let Some(bounds) = node.bounds() else {
            continue;
        };
        let entry = AppEntry {
            name: name.to_owned(),
            bounds,
        };
        if !result.iter().any(|existing| existing == &entry) {
            result.push(entry);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn 分类按钮会跳过导航标签() {
        let tree: UiNode = serde_json::from_value(json!({
            "attributes": {"type": "Root"},
            "children": [{"attributes": {"type": "List"}, "children": [
                {"attributes": {"type": "Button", "bounds": "[0,0][20,20]"}, "children": [
                    {"attributes": {"type": "Text", "text": "分类"}, "children": []}
                ]},
                {"attributes": {"type": "Button", "bounds": "[0,20][20,40]"}, "children": [
                    {"attributes": {"type": "Text", "text": "工具"}, "children": []}
                ]}
            ]}]
        }))
        .unwrap();
        let buttons = category_buttons(&tree);
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0].name, "工具");
    }

    #[test]
    fn 应用快照选择应用最多的列表() {
        let tree: UiNode = serde_json::from_value(json!({
            "attributes": {"type": "Root"},
            "children": [
                {"attributes": {"type": "List"}, "children": [
                    {"attributes": {"key": "app_name", "text": "A", "bounds": "[0,0][10,10]"}, "children": []}
                ]},
                {"attributes": {"type": "List"}, "children": [
                    {"attributes": {"key": "app_name", "text": "A", "bounds": "[0,0][10,10]"}, "children": []},
                    {"attributes": {"key": "app_name", "text": "B", "bounds": "[0,10][10,20]"}, "children": []}
                ]}
            ]
        }))
        .unwrap();
        assert_eq!(
            app_snapshot(&tree)
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["A", "B"]
        );
    }
}
