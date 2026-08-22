use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const STATE_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SearchState {
    version: u32,
    collection_complete: bool,
    app_names: Vec<String>,
    searched_names: Vec<String>,
}

impl Default for SearchState {
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
    pub(crate) fn collection_complete(&self) -> bool {
        self.collection_complete
    }

    pub(crate) fn set_collection_complete(&mut self) {
        self.collection_complete = true;
    }

    pub(crate) fn app_count(&self) -> usize {
        self.app_names.len()
    }

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

    pub(crate) fn mark_searched(&mut self, name: String) {
        if !self.searched_names.iter().any(|item| item == &name) {
            self.searched_names.push(name);
        }
    }

    pub(crate) fn pending_names(&self) -> Vec<String> {
        let searched: HashSet<&str> = self.searched_names.iter().map(String::as_str).collect();
        self.app_names
            .iter()
            .filter(|name| !searched.contains(name.as_str()))
            .cloned()
            .collect()
    }

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

#[derive(Clone, Debug)]
pub struct SearchStateStore {
    path: PathBuf,
}

impl SearchStateStore {
    pub fn for_device(serial: &str, random_mode: bool) -> Self {
        let safe_serial = sanitize_serial(serial);
        let suffix = if random_mode { ".random" } else { "" };
        Self {
            path: PathBuf::from(".cache")
                .join("search")
                .join(format!("{safe_serial}{suffix}.json")),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

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
