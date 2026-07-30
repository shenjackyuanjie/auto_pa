use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use hm_driver_rs::{
    AppIdentifier, Bounds, DeviceSelector, DeviceSerial, DeviceStatus, HdcConfig, HmDriver,
    MatchPattern, Selector, SwipeArea, SwipeDirection, UiNode,
};
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tokio::task::JoinSet;
use tokio::time::{Instant, sleep};
use tracing::{debug, error, info, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::writer::MakeWriterExt;

const APPGALLERY_BUNDLE: &str = "com.huawei.hmsapp.appgallery";
const APPGALLERY_ABILITY: &str = "MainAbility";
const FRESH_APPS_TEXT: &str = "新鲜应用";
const STATE_VERSION: u32 = 1;

const SEARCH_FIELD_KEY_PREFIX: &str = "__SearchField__search_box";
const SEARCH_BUTTON_KEY_PREFIX: &str = "__SearchField__Button__search_box";
const SEARCH_RESULT_BACK_KEY: &str = "SearchInputCard.Button.searchFrameBack";

const MAX_SCROLLS: usize = 100;
const MAX_CATEGORY_SCROLLS: usize = 100;
const SCROLL_BATCH_SIZE: usize = 2;
const SCROLL_WAIT: Duration = Duration::from_millis(100);
const MAX_SEARCH_ATTEMPTS: usize = 3;
const APP_STOP_SETTLE: Duration = Duration::from_secs(1);
const APP_START_SETTLE: Duration = Duration::from_secs(3);
const PAGE_CLICK_SETTLE: Duration = Duration::from_millis(750);
const CATEGORY_CLICK_SETTLE: Duration = Duration::from_secs(1);
const CATEGORY_SCROLL_SETTLE: Duration = Duration::from_millis(850);
const BACK_SETTLE: Duration = Duration::from_millis(1500);
const SEARCH_INPUT_FOCUS_SETTLE: Duration = Duration::from_millis(200);
const SEARCH_INPUT_SETTLE: Duration = Duration::from_millis(300);
const SEARCH_CLICK_SETTLE: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, Parser)]
#[command(
    name = "auto-pa-search",
    about = "Collect AppGallery category app names and search them one by one"
)]
struct Cli {
    /// Discard saved progress and start from the beginning.
    #[arg(long)]
    fresh: bool,

    /// Collect only fresh apps and search them in random order.
    #[arg(long)]
    random: bool,

    /// Enable verbose logging.
    #[arg(short, long)]
    verbose: bool,

    /// Disable log file output.
    #[arg(long)]
    disable_log_file: bool,

    /// Optional explicit HDC executable path.
    #[arg(long)]
    hdc_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct SearchState {
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
    fn add_apps<I>(&mut self, names: I) -> usize
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

    fn mark_searched(&mut self, name: String) {
        if !self.searched_names.iter().any(|item| item == &name) {
            self.searched_names.push(name);
        }
    }

    fn pending_names(&self) -> Vec<String> {
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
struct SearchStateStore {
    path: PathBuf,
}

impl SearchStateStore {
    fn for_device(serial: &str, random_mode: bool) -> Self {
        let safe_serial = sanitize_serial(serial);
        let suffix = if random_mode { ".random" } else { "" };
        Self {
            path: PathBuf::from(".cache")
                .join("search")
                .join(format!("{safe_serial}{suffix}.json")),
        }
    }

    fn load(&self, fresh: bool) -> Result<SearchState> {
        if fresh || !self.path.exists() {
            return Ok(SearchState::default());
        }
        let raw = fs::read_to_string(&self.path)
            .with_context(|| format!("无法读取搜索进度 [{}]", self.path.display()))?;
        let state: SearchState = serde_json::from_str(&raw)
            .with_context(|| format!("搜索进度 JSON 无效 [{}]", self.path.display()))?;
        state.validate()
    }

    fn save(&self, state: &SearchState) -> Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| anyhow!("搜索进度路径没有父目录"))?;
        fs::create_dir_all(parent)?;
        let temp_path = self.path.with_extension("json.tmp");
        let raw = serde_json::to_string_pretty(state)?;
        fs::write(&temp_path, raw)?;

        // Windows cannot rename over an existing file. The target is the
        // per-device state file created by this process, so replacement is safe.
        if self.path.exists() {
            fs::remove_file(&self.path)?;
        }
        fs::rename(&temp_path, &self.path)?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AppEntry {
    name: String,
    bounds: Bounds,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CategoryButton {
    name: String,
    bounds: Bounds,
}

struct SearchFlow {
    driver: HmDriver,
    state: SearchState,
    store: SearchStateStore,
    random_mode: bool,
    bundle: AppIdentifier,
    home_ready: bool,
    device_label: String,
}

impl SearchFlow {
    fn new(
        driver: HmDriver,
        state: SearchState,
        store: SearchStateStore,
        random_mode: bool,
        device_label: String,
    ) -> Result<Self> {
        Ok(Self {
            driver,
            state,
            store,
            random_mode,
            bundle: AppIdentifier::new(APPGALLERY_BUNDLE)?,
            home_ready: false,
            device_label,
        })
    }

    async fn run(&mut self) -> Result<()> {
        if !self.state.collection_complete {
            info!(device = %self.device_label, "开始初始分类遍历");
            self.collect_all_categories().await?;
            self.state.collection_complete = true;
            self.store.save(&self.state)?;
            info!(
                device = %self.device_label,
                total = self.state.app_names.len(),
                "名称收集完成"
            );
        } else {
            info!(device = %self.device_label, "已有完整收集进度，跳过初始遍历");
        }

        self.search_pending().await?;

        // A completed search is followed by exactly one refresh collection.
        info!(device = %self.device_label, "上一轮搜索完成，开始刷新遍历");
        self.collect_all_categories().await?;
        self.state.collection_complete = true;
        self.store.save(&self.state)?;
        self.search_pending().await?;

        info!(device = %self.device_label, "搜索流程完成");
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        info!(device = %self.device_label, "关闭 AppGallery");
        let stop_result = self.driver.stop_app(&self.bundle).await;
        let close_result = self.driver.close().await;
        stop_result.context("关闭 AppGallery 失败")?;
        close_result.context("关闭 HmDriver 失败")?;
        info!(device = %self.device_label, "设备清理完成");
        Ok(())
    }

    async fn collect_all_categories(&mut self) -> Result<()> {
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

    async fn start_appgallery(&mut self) -> Result<()> {
        info!(device = %self.device_label, "正在关闭 AppGallery");
        if let Err(error) = self.driver.stop_app(&self.bundle).await {
            warn!(device = %self.device_label, error = %error, "关闭 AppGallery 时出现警告");
        }
        sleep(APP_STOP_SETTLE).await;
        info!(device = %self.device_label, "正在启动 AppGallery");
        self.driver
            .start_app(&self.bundle, Some(APPGALLERY_ABILITY))
            .await
            .context("启动 AppGallery 失败")?;
        let ready = self
            .wait_for_appgallery(Duration::from_secs(15))
            .await
            .context("等待 AppGallery 前台超时")?;
        if !ready {
            bail!("等待 AppGallery 前台超时");
        }
        sleep(APP_START_SETTLE).await;
        info!(device = %self.device_label, "AppGallery 启动完成");
        self.home_ready = false;
        Ok(())
    }

    async fn wait_for_appgallery(&self, timeout: Duration) -> Result<bool> {
        let deadline = Instant::now() + timeout;
        loop {
            let output = self
                .driver
                .raw_shell("aa dump -l")
                .await
                .context("查询 AppGallery 前台状态失败")?;
            if foreground_bundle_present(&output.stdout, self.bundle.as_str()) {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            sleep(Duration::from_millis(250)).await;
        }
    }

    async fn click_app_or_game(&self, page: &str) -> Result<()> {
        let key = match page {
            "应用" => Some("BadgeImage.sys.symbol.bag_fill"),
            "游戏" => Some("BadgeImage.sys.symbol.game_fill"),
            _ => None,
        };
        self.click_local(
            move |node| {
                node.attribute("text").as_deref() == Some(page)
                    || key.is_some_and(|key| node.attribute("key").as_deref() == Some(key))
            },
            &format!("{page}页签"),
            Duration::from_secs(12),
        )
        .await?;
        sleep(PAGE_CLICK_SETTLE).await;
        debug!(device = %self.device_label, page, "页面入口点击完成");
        Ok(())
    }

    async fn click_categories_tab(&self) -> Result<()> {
        self.click_local(
            |node| {
                let text = node.attribute("text");
                let key = node.attribute("key");
                text.as_deref() == Some("分类")
                    || matches!(
                        key.as_deref(),
                        Some("Paf_Lantern_Button_Index_1")
                            | Some("Paf_Lantern_Text_1")
                            | Some("Paf_Lantern_Normal_Image_1")
                            | Some("Paf_Lantern_Select_Image_1")
                    )
            },
            "分类入口",
            Duration::from_secs(12),
        )
        .await?;
        sleep(PAGE_CLICK_SETTLE).await;
        debug!(device = %self.device_label, "分类入口点击完成");
        self.wait_for_categories(Duration::from_secs(12)).await?;
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
            let initial = self.driver.ui_tree().await?;
            self.click_fresh_apps(initial).await?;
            let app_layout = self
                .driver
                .wait_for_ui(Duration::from_secs(12), |tree| {
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
            let current = self.driver.ui_tree().await?;
            let names = self
                .collect_category_app_list(current, &button.name)
                .await?;
            self.back_to_categories(1).await?;
            self.save_collected_names(&button.name, names)?;
        }
        Ok(())
    }

    async fn click_fresh_apps(&self, initial: UiNode) -> Result<()> {
        info!(device = %self.device_label, "查找新鲜应用入口");
        let node = initial
            .find(|node| node.attribute("text").as_deref() == Some(FRESH_APPS_TEXT))
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
            total = self.state.app_names.len(),
            "分类收集完成"
        );
        Ok(())
    }

    async fn collect_app_list(&self, initial: UiNode, allow_empty: bool) -> Result<Vec<String>> {
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
                tokio::time::sleep(SCROLL_WAIT).await;
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
                tree = self.driver.ui_tree().await?;
            }
        }
        Ok(names)
    }

    async fn back_to_categories(&self, count: usize) -> Result<()> {
        for index in 0..count {
            self.driver.go_back().await?;
            sleep(BACK_SETTLE).await;
            if index + 1 == count {
                self.wait_for_categories(Duration::from_secs(15)).await?;
            }
        }
        Ok(())
    }

    async fn wait_for_categories(&self, timeout: Duration) -> Result<UiNode> {
        self.driver
            .wait_for_ui(timeout, |tree| !category_buttons(tree).is_empty())
            .await
            .context("等待分类列表超时")
    }

    async fn scroll_up(&self) -> Result<()> {
        self.driver
            .swipe_direction(SwipeDirection::Up, SwipeArea::FullScreen, 0.7, 2_000)
            .await
            .context("滚动 UI 失败")
    }

    async fn search_pending(&mut self) -> Result<()> {
        let mut pending = self.state.pending_names();
        if pending.is_empty() {
            info!(device = %self.device_label, "所有应用名称均已搜索完成");
            return Ok(());
        }
        if self.random_mode {
            pending.shuffle(&mut rand::rng());
        }

        self.ensure_search_home().await?;
        let total = pending.len();
        info!(
            device = %self.device_label,
            pending = total,
            "开始搜索应用名称"
        );
        let mut failed = Vec::new();
        for (index, app_name) in pending.into_iter().enumerate() {
            let mut success = false;
            for attempt in 1..=MAX_SEARCH_ATTEMPTS {
                info!(
                    device = %self.device_label,
                    app = %app_name,
                    progress = format_args!("{}/{}", index + 1, total),
                    attempt,
                    "搜索应用"
                );
                match self.search_once(&app_name).await {
                    Ok(result_count) => {
                        self.state.mark_searched(app_name.clone());
                        self.store.save(&self.state)?;
                        info!(app = %app_name, results = result_count, "搜索完成");
                        success = true;
                        break;
                    }
                    Err(error) => {
                        warn!(app = %app_name, attempt, error = %error, "搜索失败");
                        self.home_ready = false;
                        if attempt < MAX_SEARCH_ATTEMPTS {
                            self.ensure_search_home().await?;
                        }
                    }
                }
            }
            if !success {
                failed.push(app_name);
            }
        }

        if failed.is_empty() {
            info!(device = %self.device_label, "本轮应用搜索全部完成");
            Ok(())
        } else {
            error!(
                device = %self.device_label,
                failed = failed.len(),
                names = %failed.join(", "),
                "本轮应用搜索存在失败项"
            );
            bail!(
                "有 {} 个应用搜索失败，下次运行会重试：{}",
                failed.len(),
                failed.join(", ")
            )
        }
    }

    async fn ensure_search_home(&mut self) -> Result<()> {
        if self.home_ready {
            return Ok(());
        }
        info!(device = %self.device_label, "准备应用搜索主页");
        self.start_appgallery().await?;
        self.click_app_or_game("应用").await?;
        self.wait_for_key_element(SEARCH_FIELD_KEY_PREFIX, Duration::from_secs(12))
            .await?;
        self.home_ready = true;
        info!(device = %self.device_label, "应用搜索主页就绪");
        Ok(())
    }

    async fn search_once(&mut self, app_name: &str) -> Result<usize> {
        let search_field = self
            .wait_for_key_element(SEARCH_FIELD_KEY_PREFIX, Duration::from_secs(12))
            .await?;
        search_field.click().await?;
        sleep(SEARCH_INPUT_FOCUS_SETTLE).await;
        search_field.clear_text().await?;
        search_field.input_text(app_name).await?;
        sleep(SEARCH_INPUT_SETTLE).await;

        let search_button = self
            .wait_for_key_element(SEARCH_BUTTON_KEY_PREFIX, Duration::from_secs(5))
            .await?;
        search_button.click().await?;
        sleep(SEARCH_CLICK_SETTLE).await;

        let result_page = self
            .wait_local_key(
                SEARCH_RESULT_BACK_KEY,
                Duration::from_secs(15),
                "搜索结果页",
            )
            .await?;
        let result_count = match self
            .driver
            .wait_for_ui(Duration::from_secs(6), |tree| {
                !app_snapshot(tree).is_empty()
            })
            .await
        {
            Ok(layout) => self.collect_app_list(layout, false).await?.len(),
            Err(_) => {
                debug!(app = %app_name, "搜索结果页没有应用卡片");
                let _ = result_page;
                0
            }
        };

        self.click_local_key(
            SEARCH_RESULT_BACK_KEY,
            Duration::from_secs(8),
            "搜索结果页返回按钮",
        )
        .await?;
        sleep(SEARCH_CLICK_SETTLE).await;
        self.wait_for_key_element(SEARCH_FIELD_KEY_PREFIX, Duration::from_secs(15))
            .await?;
        self.home_ready = true;
        Ok(result_count)
    }

    async fn wait_for_key_element(
        &self,
        key_prefix: &str,
        timeout: Duration,
    ) -> Result<hm_driver_rs::Element> {
        let selector = Selector::new().key(MatchPattern::StartsWith(key_prefix.to_owned()));
        self.driver
            .wait_for(&selector, timeout)
            .await
            .with_context(|| format!("等待控件 [{}] 超时", key_prefix))
    }

    async fn wait_local_key(
        &self,
        key: &str,
        timeout: Duration,
        description: &str,
    ) -> Result<UiNode> {
        self.driver
            .wait_for_ui(timeout, move |node| {
                node.attribute("key").as_deref() == Some(key)
            })
            .await
            .with_context(|| format!("等待 [{}] 超时", description))
    }

    async fn click_local_key(
        &self,
        key: &str,
        timeout: Duration,
        description: &str,
    ) -> Result<UiNode> {
        self.click_local(
            move |node| node.attribute("key").as_deref() == Some(key),
            description,
            timeout,
        )
        .await
    }

    async fn click_local<F>(
        &self,
        predicate: F,
        description: &str,
        timeout: Duration,
    ) -> Result<UiNode>
    where
        F: Fn(&UiNode) -> bool,
    {
        let node = self
            .driver
            .wait_for_ui(timeout, predicate)
            .await
            .with_context(|| format!("等待 [{}] 超时", description))?;
        let bounds = node
            .bounds()
            .ok_or_else(|| anyhow!("控件 [{}] 没有有效 bounds", description))?;
        self.driver.click(bounds.center()).await?;
        Ok(node)
    }
}

fn category_buttons(tree: &UiNode) -> Vec<CategoryButton> {
    let mut best = Vec::new();
    let mut best_score = 0usize;
    let list_nodes = tree.find_all(|node| node.node_type().as_deref() == Some("List"));

    for list in list_nodes {
        let mut buttons = Vec::new();
        let mut unique_names = HashSet::new();
        for button in list.find_all(|node| node.node_type().as_deref() == Some("Button")) {
            let Some(text_node) =
                button.find(|node| node.attribute("text").is_some_and(|text| !text.is_empty()))
            else {
                continue;
            };
            let Some(name) = text_node.attribute("text") else {
                continue;
            };
            if is_category_tab_or_action(&name) || !unique_names.insert(name.clone()) {
                continue;
            }
            let Some(bounds) = button.bounds().or_else(|| text_node.bounds()) else {
                continue;
            };
            buttons.push(CategoryButton { name, bounds });
        }
        if unique_names.len() >= best_score {
            best_score = unique_names.len();
            best = buttons;
        }
    }
    best
}

fn is_category_tab_or_action(value: &str) -> bool {
    matches!(
        value,
        "精选" | "分类" | "排行榜" | "重磅更新" | "安装" | "打开" | "更新"
    )
}

fn app_snapshot(tree: &UiNode) -> Vec<AppEntry> {
    let list_nodes = tree.find_all(|node| node.node_type().as_deref() == Some("List"));
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

fn app_entries(root: &UiNode) -> Vec<AppEntry> {
    let mut result = Vec::new();
    for node in root.find_all(|node| node.attribute("key").as_deref() == Some("app_name")) {
        let Some(name) = node.attribute("text") else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let Some(bounds) = node.bounds() else {
            continue;
        };
        let entry = AppEntry { name, bounds };
        if !result.iter().any(|existing| existing == &entry) {
            result.push(entry);
        }
    }
    result
}

fn sanitize_serial(serial: &str) -> String {
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

fn foreground_bundle_present(output: &str, bundle: &str) -> bool {
    let bundle_marker = format!("bundle name [{bundle}]");
    output.split("Mission ID #").any(|mission| {
        let is_foreground = mission
            .lines()
            .any(|line| line.trim_start().starts_with("state #FOREGROUND"));
        is_foreground && mission.contains(&bundle_marker)
    })
}

async fn run_device(index: usize, serial: String, hdc_config: HdcConfig, cli: Cli) -> Result<()> {
    let device_label = format!("device-{index}");
    let store = SearchStateStore::for_device(&serial, cli.random);
    let state = store.load(cli.fresh)?;
    if cli.fresh {
        store.save(&state)?;
    }

    info!(
        device = %device_label,
        serial = %serial,
        state = %store.path.display(),
        random = cli.random,
        fresh = cli.fresh,
        "设备信息"
    );
    let driver = HmDriver::builder()
        .device(DeviceSelector::Serial(DeviceSerial::new(serial)))
        .hdc_config(hdc_config)
        .connect()
        .await
        .context("连接 HmDriver 失败")?;
    info!(device = %device_label, "HmDriver 连接成功");
    let mut flow = SearchFlow::new(
        driver,
        state.clone(),
        store.clone(),
        cli.random,
        device_label.clone(),
    )?;
    let run_result = flow.run().await;
    let cleanup_result = flow.shutdown().await;
    match (run_result, cleanup_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(error.context(format!("同时清理失败：{cleanup}"))),
    }
}

async fn run(cli: Cli) -> Result<()> {
    let hdc_config = cli
        .hdc_path
        .clone()
        .map_or_else(HdcConfig::default, |path| {
            HdcConfig::default().with_path(path)
        });
    let descriptors = HmDriver::discover_devices(hdc_config.clone())
        .await
        .context("发现设备失败")?;
    let online: Vec<String> = descriptors
        .into_iter()
        .filter_map(|descriptor| {
            if descriptor.status == DeviceStatus::Online {
                Some(descriptor.serial.expose_secret().to_owned())
            } else {
                None
            }
        })
        .collect();
    if online.is_empty() {
        bail!("未发现在线 HarmonyOS 设备");
    }
    info!(devices = online.len(), "发现在线 HarmonyOS 设备");
    for (index, serial) in online.iter().enumerate() {
        info!(device = format_args!("device-{index}"), serial, "发现设备");
    }

    let mut tasks = JoinSet::new();
    for (index, serial) in online.into_iter().enumerate() {
        tasks.spawn(run_device(index, serial, hdc_config.clone(), cli.clone()));
    }

    let mut failures = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => failures.push(error.to_string()),
            Err(error) => failures.push(format!("设备任务异常：{error}")),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        bail!(
            "{} 台设备执行失败：{}",
            failures.len(),
            failures.join(" | ")
        )
    }
}

fn init_logging(cli: &Cli) -> Option<WorkerGuard> {
    let level = if cli.verbose {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    if cli.disable_log_file {
        tracing_subscriber::fmt()
            .with_max_level(level)
            .with_target(false)
            .init();
        return None;
    }

    let appender = tracing_appender::rolling::daily("logs", "search-rust.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let writer = std::io::stdout.and(writer);
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        .with_writer(writer)
        .init();
    Some(guard)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let _log_guard = init_logging(&cli);
    run(cli).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn state_adds_only_unique_names() {
        let mut state = SearchState::default();
        assert_eq!(state.add_apps(["A".into(), "A".into(), "B".into()]), 2);
        assert_eq!(state.add_apps(["B".into(), "C".into()]), 1);
        assert_eq!(state.app_names, vec!["A", "B", "C"]);
    }

    #[test]
    fn pending_names_exclude_searched_names() {
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
    fn state_json_remains_compatible() {
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
    fn serial_path_sanitization_matches_python_shape() {
        assert_eq!(sanitize_serial("a:b/._"), "a_b");
        assert_eq!(sanitize_serial("...___"), "device");
    }

    #[test]
    fn app_snapshot_chooses_the_list_with_most_apps() {
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
        let snapshot = app_snapshot(&tree);
        assert_eq!(
            snapshot
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["A", "B"]
        );
    }

    #[test]
    fn category_buttons_skip_navigation_tabs() {
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
    fn foreground_bundle_search_checks_all_missions() {
        let output = "Mission ID #1\n  bundle name [com.example.first]\n  state #FOREGROUND\nMission ID #2\n  bundle name [com.huawei.hmsapp.appgallery]\n  state #FOREGROUND\n";
        assert!(foreground_bundle_present(output, APPGALLERY_BUNDLE));
    }
}
