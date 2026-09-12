//! `search` 子命令的设备入口与状态机。
//!
//! [`SearchFlow`] 把「分类遍历收集名称」和「逐个搜索名称」串成一条流水线：先做一次分类
//! 遍历，再搜索全部待搜索名称，随后固定再做一轮刷新遍历，最后只搜索刷新后新出现的名称
//! （首轮搜索失败、因而没有被标记为已搜索的名称也会在这一轮重试）。进度由 [`SearchState`]
//! 记录、[`SearchStateStore`] 落盘，中断后重跑可以接着上次的位置继续。
//!
//! AppGallery 在平板上走平板/PC 布局，且部分 Hypium Agent 没有实现远端按 key 查找的
//! 选择器，因此这里的控件识别全部基于 `dumpLayout` 的本地快照（key、text、bounds）：
//! 先等到满足条件的整棵 UI 树，再在树里定位节点、按坐标点击或滑动。
//!
//! [`run_device`] 是每台在线设备一个任务时的入口：加载进度、连接 [`HmDriver`]，并保证
//! 无论 [`SearchFlow::run`] 成功与否都执行清理。

use anyhow::{Context, Result, anyhow, bail};
use hm_driver_rs::{
    AppIdentifier, DeviceSelector, DeviceSerial, HdcConfig, HmDriver, SwipeArea, SwipeDirection,
    UiNode,
};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};

use crate::appgallery::{APPGALLERY_ABILITY, APPGALLERY_BUNDLE, category_buttons};
use crate::search::state::{SearchState, SearchStateStore};

/// 分类列表滚动遍历的硬上限：超过它仍未到底说明页面结构已经不符合预期，
/// 直接报错比继续滑动（或返回一份不完整的分类表）更安全。
pub(crate) const MAX_CATEGORY_SCROLLS: usize = 100;
/// 关闭 AppGallery 之后、重新启动之前的等待，留给上一次会话退出。
pub(crate) const APP_STOP_SETTLE: Duration = Duration::from_secs(1);
/// 确认 AppGallery 已到前台之后的额外等待，留给首页把内容渲染出来。
pub(crate) const APP_START_SETTLE: Duration = Duration::from_secs(3);
/// 切换「应用」/「游戏」页签、或刚进入分类页之后的等待。
pub(crate) const PAGE_CLICK_SETTLE: Duration = Duration::from_millis(750);
/// 点击分类按钮、以及点开「新鲜应用」入口之后的等待。
pub(crate) const CATEGORY_CLICK_SETTLE: Duration = Duration::from_secs(1);
/// 分类页每滚动一屏之后的等待，让列表把下一屏内容渲染出来再取快照。
pub(crate) const CATEGORY_SCROLL_SETTLE: Duration = Duration::from_millis(850);
/// 进入分类之后，等待「新鲜应用」入口或应用列表出现的最长时间。
pub(crate) const CATEGORY_CONTENT_TIMEOUT: Duration = Duration::from_secs(5);
/// 每次 `go_back` 之后的等待；返回动画比普通点击慢，等待过短会读到上一页的树。
pub(crate) const BACK_SETTLE: Duration = Duration::from_millis(1500);
/// 应用详情页每滚动一屏后的等待时间。
pub(crate) const DETAIL_SCROLL_SETTLE: Duration = Duration::from_millis(700);

/// 单台设备上的一轮搜索：持有设备会话与进度，并按固定阶段推进。
pub struct SearchFlow {
    /// 设备会话；点击、滑动、`dumpLayout` 都经它执行。
    pub(crate) driver: HmDriver,
    /// 内存中的名称集合与已搜索集合，收集和搜索都先写这里。
    pub(crate) state: SearchState,
    /// 进度落盘：每收集完一个分类、每成功搜索一个名称后保存，供中断续跑。
    pub(crate) store: SearchStateStore,
    /// `--random`：只走「应用」页签并收集每个分类的「新鲜应用」子页面，搜索顺序随机打乱，
    /// 进度文件也会带 `.random` 后缀与默认模式区分开。
    pub(crate) random_mode: bool,
    /// 是否执行「打开第一个搜索结果、收集同开发者应用」这一步（`--skip-developer` 时关闭）。
    pub(crate) developer_scan: bool,
    /// AppGallery 的应用标识，用于启停应用。
    pub(crate) bundle: AppIdentifier,
    /// 搜索主页（「应用」页签 + 搜索框）是否已经就绪；搜索失败时置回 `false` 触发重新准备，
    /// 避免每个名称都重启一次 AppGallery。
    pub(crate) home_ready: bool,
    /// 日志里的设备标签，形如 `device-0`。
    pub(crate) device_label: String,
}

impl SearchFlow {
    pub fn new(
        driver: HmDriver,
        state: SearchState,
        store: SearchStateStore,
        random_mode: bool,
        developer_scan: bool,
        device_label: String,
    ) -> Result<Self> {
        Ok(Self {
            driver,
            state,
            store,
            random_mode,
            developer_scan,
            bundle: AppIdentifier::new(APPGALLERY_BUNDLE)?,
            home_ready: false,
            device_label,
        })
    }

    /// 执行四个阶段：初始分类遍历 -> 首轮搜索 -> 固定刷新一轮 -> 再搜索。
    ///
    /// 初始遍历只在进度里还没有 `collection_complete` 标记时执行，因此中断后重跑会跳过
    /// 它、直接搜索已有名称。首轮搜索的失败只记录 warning 就继续：失败名称没有被标记为
    /// 已搜索，会和刷新遍历新发现的名称一起在最后一轮重试。刷新遍历与最后一轮搜索则用 `?`
    /// 直接结束流程——前者失败说明页面结构已经异常，后者已经是最后一步，都没有继续的意义。
    pub async fn run(&mut self) -> Result<()> {
        if !self.state.collection_complete() {
            info!(device = %self.device_label, "开始初始分类遍历");
            self.collect_all_categories().await?;
            self.state.set_collection_complete();
            self.store.save(&self.state)?;
            info!(
                device = %self.device_label,
                total = self.state.app_count(),
                "名称收集完成"
            );
        } else {
            info!(device = %self.device_label, "已有完整收集进度，跳过初始遍历");
        }

        // 首轮搜索失败时保留进度并继续刷新抓取。失败名称没有被标记为已搜索，
        // 因而会与刷新后新增的名称一起在下一轮重新尝试。
        if let Err(error) = self.search_pending().await {
            warn!(
                device = %self.device_label,
                error = %error,
                "首轮搜索存在失败，继续刷新分类"
            );
        }

        // 首轮搜索完成（或暂时失败）后，固定再执行一次刷新抓取。
        info!(device = %self.device_label, "上一轮搜索完成，开始刷新遍历");
        self.collect_all_categories().await?;
        self.state.set_collection_complete();
        self.store.save(&self.state)?;
        self.search_pending().await?;

        info!(device = %self.device_label, "搜索流程完成");
        Ok(())
    }

    /// 关闭 AppGallery 与 HmDriver。
    ///
    /// 两次关闭都会尝试执行，第一步失败也不会跳过第二步；错误按「先 AppGallery、
    /// 后 HmDriver」的顺序返回。
    pub async fn shutdown(&self) -> Result<()> {
        info!(device = %self.device_label, "关闭 AppGallery");
        let stop_result = self.driver.stop_app(&self.bundle).await;
        let close_result = self.driver.close().await;
        stop_result.context("关闭 AppGallery 失败")?;
        close_result.context("关闭 HmDriver 失败")?;
        info!(device = %self.device_label, "设备清理完成");
        Ok(())
    }

    /// 冷启动 AppGallery 并确认它已到前台。
    ///
    /// 先 `stop_app` 再启动，避免复用上一次残留的页面状态；关闭失败只记 warning，因为
    /// 应用本来就可能没在运行。启动后把 `home_ready` 复位，让搜索重新定位主页。
    pub(crate) async fn start_appgallery(&mut self) -> Result<()> {
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
        self.driver
            .wait_for_app(&self.bundle, timeout)
            .await
            .context("查询 AppGallery 前台状态失败")
    }

    /// 切换到「应用」或「游戏」页签。
    ///
    /// 页签文字不一定挂在可点击节点上，所以除文字外还接受页签图标的 key；传入其它值时
    /// 只按文字匹配。
    pub(crate) async fn click_app_or_game(&self, page: &str) -> Result<()> {
        let key = match page {
            "应用" => Some("BadgeImage.sys.symbol.bag_fill"),
            "游戏" => Some("BadgeImage.sys.symbol.game_fill"),
            _ => None,
        };
        self.click_local(
            move |node| {
                node.attribute_str("text") == Some(page)
                    || key.is_some_and(|key| node.attribute_str("key") == Some(key))
            },
            &format!("{page}页签"),
            Duration::from_secs(12),
        )
        .await?;
        sleep(PAGE_CLICK_SETTLE).await;
        Ok(())
    }

    /// 点开底部「分类」页签，并等到分类列表出现才算切换完成。
    ///
    /// 「分类」入口在布局里可能只有文字，也可能只暴露 `Paf_Lantern_*` 系列 key，两种条件
    /// 都要匹配。
    pub(crate) async fn click_categories_tab(&self) -> Result<()> {
        self.click_local(
            |node| {
                let text = node.attribute_str("text");
                let key = node.attribute_str("key");
                text == Some("分类")
                    || matches!(
                        key,
                        Some(
                            "Paf_Lantern_Button_Index_1"
                                | "Paf_Lantern_Text_1"
                                | "Paf_Lantern_Normal_Image_1"
                                | "Paf_Lantern_Select_Image_1"
                        )
                    )
            },
            "分类入口",
            Duration::from_secs(12),
        )
        .await?;
        sleep(PAGE_CLICK_SETTLE).await;
        self.wait_for_categories(Duration::from_secs(12)).await?;
        Ok(())
    }

    /// 连续返回 `count` 级，并且只在最后一级等待分类列表。
    ///
    /// 中间层级不等待，避免返回动画还没结束就去判断页面状态；最后一级必须重新出现分类
    /// 按钮，才能确认后续的分类遍历可以继续。
    pub(crate) async fn back_to_categories(&self, count: usize) -> Result<()> {
        for index in 0..count {
            self.driver.go_back().await?;
            sleep(BACK_SETTLE).await;
            if index + 1 == count {
                self.wait_for_categories(Duration::from_secs(15)).await?;
            }
        }
        Ok(())
    }

    /// 等当前页面出现分类按钮，并返回那一刻的整棵 UI 树。
    ///
    /// 用 `wait_for_ui_tree` 而不是 `wait_for_ui`：分类按钮的数量和文本要在完整树里交给
    /// [`category_buttons`] 重新计算，只拿到单个节点没有用。
    pub(crate) async fn wait_for_categories(&self, timeout: Duration) -> Result<UiNode> {
        self.driver
            .wait_for_ui_tree(timeout, |tree| !category_buttons(tree).is_empty())
            .await
            .context("等待分类列表超时")
    }

    /// 全屏上滑（70% 屏高、2000 像素/秒），把列表翻到下一屏。
    pub(crate) async fn scroll_up(&self) -> Result<()> {
        self.driver
            .swipe_direction(SwipeDirection::Up, SwipeArea::FullScreen, 0.7, 2_000)
            .await
            .context("滚动 UI 失败")
    }

    /// 在超时时间内等到满足 `predicate` 的控件，并点击它的点击目标中心。
    ///
    /// 等待阶段拿到的是整棵树，所以这里再用 `find_click_target` 向上找真正带 `bounds` 的
    /// 可点击祖先——ArkUI 常把可点击性挂在容器上、文字放在子节点。`description` 只用于
    /// 错误信息。
    pub(crate) async fn click_local<F>(
        &self,
        predicate: F,
        description: &str,
        timeout: Duration,
    ) -> Result<UiNode>
    where
        F: Fn(&UiNode) -> bool,
    {
        let tree = self
            .driver
            .wait_for_ui_tree(timeout, |tree| tree.find(&predicate).is_some())
            .await
            .with_context(|| format!("等待 [{description}] 超时"))?;
        let node = tree
            .find_click_target(&predicate)
            .ok_or_else(|| anyhow!("控件 [{description}] 没有有效点击目标"))?;
        let bounds = node
            .bounds()
            .ok_or_else(|| anyhow!("控件 [{description}] 没有有效 bounds"))?;
        self.driver.click(bounds.center()).await?;
        Ok(node.clone())
    }
}

/// 单台设备的执行入口：加载进度、连接驱动、跑完整流程，最后一定清理。
///
/// `index` 只用于日志里的 `device-N` 标签，`serial` 同时决定目标设备和进度文件路径。
/// `fresh` 为真时丢弃已保存的进度，并立即写入一份空进度，避免首次收集失败后又被当成
/// 「已有进度」读回来。
///
/// 无论 [`SearchFlow::run`] 的结果如何都会调用 [`SearchFlow::shutdown`]：执行失败时
/// 优先返回执行错误，清理也失败就把清理错误作为附加上下文挂在后面，保证两类失败都不会
/// 被静默吞掉。
pub async fn run_device(
    index: usize,
    serial: String,
    hdc_config: HdcConfig,
    fresh: bool,
    random_mode: bool,
    developer_scan: bool,
) -> Result<()> {
    let device_label = format!("device-{index}");
    let store = SearchStateStore::for_device(&serial, random_mode);
    let state = store.load(fresh)?;
    if fresh {
        store.save(&state)?;
    }

    info!(
        device = %device_label,
        serial = %serial,
        state = %store.path().display(),
        random = random_mode,
        developer = developer_scan,
        fresh,
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
        state,
        store,
        random_mode,
        developer_scan,
        device_label.clone(),
    )?;
    let run_result = flow.run().await;
    let cleanup_result = flow.shutdown().await;
    match (run_result, cleanup_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => {
            error!(device = %device_label, error = ?error, "设备执行失败");
            Err(error)
        }
        (Ok(()), Err(error)) => {
            error!(device = %device_label, error = ?error, "设备清理失败");
            Err(error)
        }
        (Err(error), Err(cleanup)) => {
            error!(device = %device_label, error = ?error, cleanup = ?cleanup, "设备执行和清理均失败");
            Err(error.context(format!("同时清理失败：{cleanup}")))
        }
    }
}
