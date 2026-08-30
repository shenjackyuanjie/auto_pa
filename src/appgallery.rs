use hm_driver_rs::{Bounds, UiNode};
use std::collections::HashSet;

pub const APPGALLERY_BUNDLE: &str = "com.huawei.hmsapp.appgallery";
pub const APPGALLERY_ABILITY: &str = "MainAbility";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppEntry {
    pub name: String,
    pub bounds: Bounds,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategoryButton {
    pub name: String,
    pub bounds: Bounds,
}

/// 从所有 List 控件中选择最像分类列表的那个，并排除页面导航和操作按钮。
pub fn category_buttons(tree: &UiNode) -> Vec<CategoryButton> {
    let mut best = Vec::new();
    let mut best_score = 0usize;
    let list_nodes = tree.find_all(|node| node.attribute_str("type") == Some("List"));

    for list in list_nodes {
        let mut buttons = Vec::new();
        let mut unique_names = HashSet::new();
        for button in list.find_all(|node| node.attribute_str("type") == Some("Button")) {
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
            let Some(bounds) = button.bounds().or_else(|| text_node.bounds()) else {
                continue;
            };
            buttons.push(CategoryButton {
                name: name.to_owned(),
                bounds,
            });
        }
        if unique_names.len() >= best_score {
            best_score = unique_names.len();
            best = buttons;
        }
    }
    best
}

/// 选取应用卡片最多的 List，避免把导航栏或遮罩层误认为应用列表。
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

fn is_category_tab_or_action(value: &str) -> bool {
    matches!(
        value,
        "精选" | "分类" | "排行榜" | "重磅更新" | "安装" | "打开" | "更新"
    )
}

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
