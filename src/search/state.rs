//! 搜索流程的断点续跑进度：已收集的应用名称与已搜索标记。
//!
//! `SearchFlow` 每收集完一个分类、每成功搜索一个应用都会把进度落盘，因此进程中断后
//! 再次运行能接着上次继续：已完成的分类遍历不再重做；搜索失败的名称不会被标记为已
//! 搜索，会和新收集到的名称一起进入下一轮。
//!
//! 进度文件按设备序列号（以及 `random` 模式）分开保存：`random` 模式只收集「新鲜
//! 应用」，两种模式得到的名称集合不同，共用一份进度会互相污染。

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// 进度文件的格式版本；加载时与文件内的 `version` 比对，不一致就拒绝使用，
/// 避免用新逻辑误读旧结构。
const STATE_VERSION: u32 = 1;

/// 单个设备的搜索进度，序列化后由 [`SearchStateStore`] 读写。
///
/// 「已收集」和「已搜索」分开记录，因此不需要额外的失败列表：搜索失败只表现为
/// 名称仍在待搜索集合里。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SearchState {
    /// 进度文件格式版本，必须等于 `STATE_VERSION`。
    version: u32,
    /// 分类遍历是否已完整跑过一轮；为真时再次运行会跳过初始遍历。
    collection_complete: bool,
    /// 已收集到的应用名，按首次出现顺序排列且不重复。
    app_names: Vec<String>,
    /// 已成功搜索过的应用名；失败项不写入这里，因而会留在待搜索列表中重试。
    searched_names: Vec<String>,
}

impl Default for SearchState {
    /// 全新进度：分类遍历未完成，也没有任何名称。
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            collection_complete: false,
            app_names: Vec::new(),
            searched_names: Vec::new(),
        }
    }
}

impl SearchState {
    /// 分类遍历是否已经完整跑过一轮。
    pub(crate) fn collection_complete(&self) -> bool {
        self.collection_complete
    }

    /// 标记分类遍历已完成；调用方随后会调用 `save` 落盘，以免中断后重复遍历。
    pub(crate) fn set_collection_complete(&mut self) {
        self.collection_complete = true;
    }

    /// 已收集的应用名总数，用于日志里的进度显示。
    pub(crate) fn app_count(&self) -> usize {
        self.app_names.len()
    }

    /// 合并一批应用名，跳过空名称与已存在的名称，返回本次新增的数量。
    ///
    /// 先用现有名称建集合再逐个插入，既完成去重，又保持 `app_names` 的首次出现顺序。
    pub(crate) fn add_apps<I>(&mut self, names: I) -> usize
    where
        I: IntoIterator<Item = String>,
    {
        let before = self.app_names.len();
        let mut known: HashSet<String> = self.app_names.iter().cloned().collect();
        for name in names {
            if !name.is_empty() && known.insert(name.clone()) {
                self.app_names.push(name);
            }
        }
        self.app_names.len() - before
    }

    /// 记录一个已成功搜索的名称，重复调用不会重复记录。
    pub(crate) fn mark_searched(&mut self, name: String) {
        if !self.searched_names.iter().any(|item| item == &name) {
            self.searched_names.push(name);
        }
    }

    /// 尚未成功搜索过的应用名，顺序与 `app_names` 一致。
    pub(crate) fn pending_names(&self) -> Vec<String> {
        let searched: HashSet<&str> = self.searched_names.iter().map(String::as_str).collect();
        self.app_names
            .iter()
            .filter(|name| !searched.contains(name.as_str()))
            .cloned()
            .collect()
    }

    /// 校验反序列化得到的进度：版本不匹配或出现空名称时直接报错。
    ///
    /// 这里既不修复也不跳过：进度文件异常时提示用 `--fresh` 重来，比静默拿一份可疑
    /// 数据继续跑设备操作更安全。
    fn validate(self) -> Result<Self> {
        if self.version != STATE_VERSION {
            bail!("搜索进度文件版本不兼容，请使用 --fresh 重新开始");
        }
        if self.app_names.iter().any(String::is_empty)
            || self.searched_names.iter().any(String::is_empty)
        {
            bail!("搜索进度文件包含空应用名称");
        }
        Ok(self)
    }
}

/// 进度文件的位置：当前工作目录下的 `.cache/search/`，按设备序列号（和运行模式）命名。
#[derive(Clone, Debug)]
pub struct SearchStateStore {
    path: PathBuf,
}

impl SearchStateStore {
    /// 按设备序列号与运行模式生成进度文件路径。
    ///
    /// `random_mode` 追加 `.random` 后缀单独存一份，因为该模式只收集「新鲜应用」，
    /// 名称集合与常规模式不同。
    pub fn for_device(serial: &str, random_mode: bool) -> Self {
        let safe_serial = sanitize_serial(serial);
        let suffix = if random_mode { ".random" } else { "" };
        Self {
            path: PathBuf::from(".cache")
                .join("search")
                .join(format!("{safe_serial}{suffix}.json")),
        }
    }

    /// 进度文件路径，用于日志和错误信息。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 读取进度；`fresh` 为真或文件不存在时返回全新进度。
    ///
    /// 读取或解析失败一律报错而不是退回默认进度：磁盘上有损坏文件时从头再来会重复
    /// 大量设备操作，是否放弃已有进度应当由用户用 `--fresh` 显式决定。
    pub fn load(&self, fresh: bool) -> Result<SearchState> {
        if fresh || !self.path.exists() {
            return Ok(SearchState::default());
        }
        let raw = fs::read_to_string(&self.path)
            .with_context(|| format!("无法读取搜索进度 [{}]", self.path.display()))?;
        let state: SearchState = serde_json::from_str(&raw)
            .with_context(|| format!("搜索进度 JSON 无效 [{}]", self.path.display()))?;
        state.validate()
    }

    /// 保存进度：先写同目录下的 `*.json.tmp`，再替换正式文件。
    ///
    /// 中途被打断也只留下临时文件，正式进度文件不会变成半截 JSON。
    pub(crate) fn save(&self, state: &SearchState) -> Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| anyhow!("搜索进度路径没有父目录"))?;
        fs::create_dir_all(parent)?;
        let temp_path = self.path.with_extension("json.tmp");
        let raw = serde_json::to_string_pretty(state)?;
        fs::write(&temp_path, raw)?;

        // Windows 不能直接覆盖重命名；进度文件由当前进程独占，可以安全替换。
        if self.path.exists() {
            fs::remove_file(&self.path)?;
        }
        fs::rename(&temp_path, &self.path)?;
        Ok(())
    }
}

/// 把设备序列号转换成可以安全用作文件名的字符串。
///
/// `hdc` 返回的序列号可能带 `:`、`/` 等路径字符，直接拼进路径会指向别的位置；这里
/// 只保留 ASCII 字母、数字和 `_`、`.`、`-`，其余字符替换为 `_`，再去掉首尾的 `.`
/// 和 `_`，以免生成 `.`、`..` 这类指向目录的名字（`"a:b/._"` 会得到 `a_b`）。
/// 结果为空时回退为 `device`，保证文件名始终有效。
pub fn sanitize_serial(serial: &str) -> String {
    let mut safe: String = serial
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    safe = safe.trim_matches(['.', '_']).to_owned();
    if safe.is_empty() {
        "device".to_owned()
    } else {
        safe
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn 状态只保留唯一应用名() {
        let mut state = SearchState::default();
        assert_eq!(state.add_apps(["A".into(), "A".into(), "B".into()]), 2);
        assert_eq!(state.add_apps(["B".into(), "C".into()]), 1);
        assert_eq!(state.app_names, vec!["A", "B", "C"]);
    }

    #[test]
    fn 待搜索列表会排除已搜索名称() {
        let mut state = SearchState {
            version: STATE_VERSION,
            collection_complete: true,
            app_names: vec!["A".into(), "B".into()],
            searched_names: vec!["A".into()],
        };
        assert_eq!(state.pending_names(), vec!["B"]);
        state.mark_searched("B".into());
        assert!(state.pending_names().is_empty());
    }

    #[test]
    fn 状态文件格式保持兼容() {
        let value = json!({
            "version": 1,
            "collection_complete": true,
            "app_names": ["应用 A"],
            "searched_names": []
        });
        let state: SearchState = serde_json::from_value(value).unwrap();
        assert!(state.validate().is_ok());
    }

    #[test]
    fn 设备序列号路径保持兼容() {
        assert_eq!(sanitize_serial("a:b/._"), "a_b");
        assert_eq!(sanitize_serial("...___"), "device");
    }
}
