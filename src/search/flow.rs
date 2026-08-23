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

pub(crate) const MAX_CATEGORY_SCROLLS: usize = 100;
pub(crate) const APP_STOP_SETTLE: Duration = Duration::from_secs(1);
pub(crate) const APP_START_SETTLE: Duration = Duration::from_secs(3);
pub(crate) const PAGE_CLICK_SETTLE: Duration = Duration::from_millis(750);
pub(crate) const CATEGORY_CLICK_SETTLE: Duration = Duration::from_secs(1);
pub(crate) const CATEGORY_SCROLL_SETTLE: Duration = Duration::from_millis(850);
pub(crate) const CATEGORY_CONTENT_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const BACK_SETTLE: Duration = Duration::from_millis(1500);

pub struct SearchFlow {
    pub(crate) driver: HmDriver,
    pub(crate) state: SearchState,
    pub(crate) store: SearchStateStore,
    pub(crate) random_mode: bool,
    pub(crate) bundle: AppIdentifier,
    pub(crate) home_ready: bool,
    pub(crate) device_label: String,
}

impl SearchFlow {
    pub fn new(
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

    pub async fn shutdown(&self) -> Result<()> {
        info!(device = %self.device_label, "关闭 AppGallery");
        let stop_result = self.driver.stop_app(&self.bundle).await;
        let close_result = self.driver.close().await;
        stop_result.context("关闭 AppGallery 失败")?;
        close_result.context("关闭 HmDriver 失败")?;
        info!(device = %self.device_label, "设备清理完成");
        Ok(())
    }

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

    pub(crate) async fn wait_for_categories(&self, timeout: Duration) -> Result<UiNode> {
        self.driver
            .wait_for_ui_tree(timeout, |tree| !category_buttons(tree).is_empty())
            .await
            .context("等待分类列表超时")
    }

    pub(crate) async fn scroll_up(&self) -> Result<()> {
        self.driver
            .swipe_direction(SwipeDirection::Up, SwipeArea::FullScreen, 0.7, 2_000)
            .await
            .context("滚动 UI 失败")
    }

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

pub async fn run_device(
    index: usize,
    serial: String,
    hdc_config: HdcConfig,
    fresh: bool,
    random_mode: bool,
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
    let mut flow = SearchFlow::new(driver, state, store, random_mode, device_label.clone())?;
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
