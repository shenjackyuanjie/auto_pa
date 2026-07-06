import abc
import argparse
from collections import defaultdict
from dataclasses import dataclass
import datetime
import re
from typing import Any, Callable, Coroutine, Optional, Type
import anyio
from tianxiu2b2t.units import parse_time_units

from src.logger import logger
from src import hmgallery as gallery, hdc, utils
from tianxiu2b2t.utils import runtime
from tianxiu2b2t.units import format_count_time


@dataclass
class StorageValue:
    phone: bool = False
    tab_app_btn: Optional[str] = None
    tab_game_btn: Optional[str] = None
    app_exit_btn: Optional[str] = None
    app_share_btn: Optional[str] = None
    app_share_with_gallery_btn: Optional[str] = None
    app_share_to_gallery_btn: Optional[str] = None
    app_direct_share_to_gallery_btn: Optional[str] = None
    app_info_version: int = 0
    is_new_ui: Optional[bool] = None


@dataclass
class PullResult:
    total: int = 0
    new: int = 0

    def add(self, val: "PullResult"):
        self.total += val.total
        self.new += val.new

@dataclass
class Loop:
    max: int
    current: int = 0
    loop_wait: float = 0

APPGALLERY_PKG = "com.huawei.hmsapp.appgallery"
APPGALLERY_ABILITY = "MainAbility"
FUCKOFF_APPGALLERY_VERSION_CODE: int = 1460801300
FUCKOFF_SUB_CHUNKS = [re.compile("新鲜(应用|游戏)"), re.compile("时下畅销(应用|游戏)")]
CATEGORY_PAGE_TABS = frozenset({"精选", "分类", "排行榜", "重磅更新"})
CATEGORY_PAGE_ACTIONS = frozenset({"安装", "打开", "更新"})


def _category_list_score(layout: Any) -> int:
    category_names: set[str] = set()
    for btn_path in utils.find_json_value_as_path(layout, "Button"):
        try:
            button = utils.find_json_value_by_prev_path(layout, btn_path, 2)
            text_path = utils.find_json_value_as_path(button, "Text")[0]
            text_node = utils.find_json_value_by_prev_path(button, text_path)
            text = text_node["text"]
        except (IndexError, KeyError, TypeError):
            continue
        if (
            isinstance(text, str)
            and text
            and text not in CATEGORY_PAGE_TABS
            and text not in CATEGORY_PAGE_ACTIONS
        ):
            category_names.add(text)
    return len(category_names)


def find_categories_list(layout: Any) -> Any:
    candidates: list[tuple[int, int, Any]] = []
    for index, list_path in enumerate(utils.find_json_value_as_path(layout, "List")):
        list_layout = utils.find_json_value_by_prev_path(layout, list_path, 2)
        candidates.append((_category_list_score(list_layout), index, list_layout))

    if not candidates:
        raise RuntimeError("当前页面未找到 List 控件")

    score, _, categories_list = max(candidates, key=lambda item: (item[0], item[1]))
    if score == 0:
        raise RuntimeError("当前页面未找到有效分类列表")
    return categories_list

class AppGalleryCommonDevice(metaclass=abc.ABCMeta):
    def __init__(self, device: hdc.Device):
        self.device = device
        self.total_pull_res = PullResult()
        self.all_pulled_apps: list[str] = []
        self._process: Optional[hdc.HilogProcess] = None # type: ignore

    @property
    def process(self) -> hdc.HilogProcess:
        if self._process is None:
            raise RuntimeError("process not set")
        return self._process


    def set_process(self, process: hdc.HilogProcess):
        self._process = process

    def del_process(self):
        self._process = None

    @property
    def tag(self):
        return self.device.tag

    @property
    def display_device_id(self):
        return self.device.display_device_id

    @property
    def device_id(self) -> str:
        return self.device._device

    @property
    def connection_type(self) -> str:
        return self.device._connection_type

    @property
    def name(self) -> str:
        return self.device.name

    @property
    def main_screen(self) -> tuple[int, int]:
        return self.device.main_screen

    @property
    def device_type(self) -> str:
        return self.device.device_type

    @property
    def model(self) -> str:
        return self.device.model

    @property
    def sn(self) -> str:
        return self.device.sn

    def dump_layout_to_json(self, fuck_usb_connection: bool = True) -> Any:
        return self.device.dump_layout_to_json(fuck_usb_connection)

    def click_pos(
        self,
        x: float,
        y: float,
    ):
        return self.device.click_pos(x, y)

    def click_pos_by_scale(
        self,
        x_scale: float,
        y_scale: float,
    ):
        return self.device.click_pos_by_scale(x_scale, y_scale)

    def click_by_bounds(
        self, bounds: tuple[float, float, float, float] | str, wait_for: float = 0.75
    ):
        return self.device.click_by_bounds(bounds, wait_for)

    def roll_to_y(
        self,
        x_scale: float,
        y_scale: float,
        roll_distance: float,
        wait_for: float = 0.85,
    ):
        return self.device.roll_to_y(
            x_scale, y_scale, roll_distance, wait_for)

    def simple_roll_down(
        self, x_scale: float, y_scale: float, roll_scale: float, wait_for: float = 0.85
    ):
        return self.device.simple_roll_down(
            x_scale, y_scale, roll_scale, wait_for
        )
    async def drag_to_back(
        self,
    ):
        return self.device.drag_to_back()

    def reset_pointer(
        self,
    ):
        return self.device.reset_pointer()

    def open_app(self, package: str, ability: str):
        return self.device.open_app(package, ability)

    def close_app(self, package: str):
        return self.device.close_app(package)

    def get_bottom_bar(self):
        return self.device.get_bottom_bar()

    def find_back_bounds(self, layout: Any) -> Optional[str]:
        return self.device.find_back_bounds(layout)

    async def go_back(self, layout: Any | None = None, wait_for: float = 0.75):
        return await self.device.go_back(layout, wait_for)

    def get_app_info(self, package: str) -> Coroutine[Any, Any, Optional[hdc.AppInfo]]:
        return self.device.get_app_info(package)
    

    async def start_app(self):
        await start_app(self)

    async def go_app_page(self):
        await go_app_page(self)

    async def go_game_page(self):
        await go_game_page(self)

    async def go_categories_page(self):
        await go_categories_page(self)

    async def pull_categories(self):
        pulled_categories = []
        while 1:
            current_categories_len = len(pulled_categories)

            layout = await self.dump_layout_to_json()
            layout = find_categories_list(layout)
            # and then fuck to find btn
            btns = utils.find_json_value_as_path(layout, "Button")
            for btn_path in btns:
                try:
                    btn = utils.find_json_value_by_prev_path(layout, btn_path)
                    btn_pos = btn["bounds"]
                    txt = utils.find_json_value_by_prev_path(layout, btn_path, 2)
                    text = utils.find_json_value_by_prev_path(
                        txt, utils.find_json_value_as_path(txt, "Text")[0]
                    )["text"]
                except Exception:
                    continue
                if text in pulled_categories:
                    continue
                pulled_categories.append(text)
                logger.debug(f"[{self.tag}] [{text}] [{btn_pos}]")

                # The current tablet UI puts the page tabs in the
                # same List as the real category buttons. They are navigation controls,
                # not categories to open and crawl.
                if text in CATEGORY_PAGE_TABS:
                    logger.debug(f"[{self.tag}] 跳过分类页导航 [{text}]")
                    continue

                if text in skip_categories:
                    continue
                
                logger.info(f"[{self.tag}] 正在拉取分类 [{text}]...")
                # await self.click_by_bounds(btn_pos, 1.75)
                # await anyio.sleep(1 + ping * 0.05)
                # await start_pull_apps(device, text)
                res = await self.custom_pull_apps(btn_pos, text)
                self.total_pull_res.add(res)
            if current_categories_len == len(pulled_categories):
                break
            await self.simple_roll_down(0.5, 0.2, 0.8)

    async def get_new_ui(self) -> bool:
        if (val := global_var[self.sn].is_new_ui) is not None:
            return val
        layout = await self.dump_layout_to_json()
        val = sum([len(utils.find_json_value_as_path(layout, chunk)) for chunk in FUCKOFF_SUB_CHUNKS]) != 0
        global_var[self.sn].is_new_ui = val
        return val

    @abc.abstractmethod
    async def custom_pull_apps(self, btn_pos: str, category: str) -> PullResult:
        raise NotImplementedError
    
    @abc.abstractmethod
    async def share_app(self, app: str) -> bool:
        raise NotImplementedError

    @abc.abstractmethod
    async def share_apps(self, apps: list[str], apps_pos: dict[str, str]) -> list[str]:
        pending_new_apps = await gallery.get_gallery().get_not_exists_apps(apps)
        for app in pending_new_apps:
            logger.success(f"[{self.tag}] 发现新应用 [{app}]")
            await self.click_by_bounds(apps_pos[app], 1 + ping * 0.05)
            # detail
            await self.share_app(app)
        return pending_new_apps

    

    async def start_pull_apps(self):
        apps = []
        bottom_bar = await self.get_bottom_bar()
        while 1:
            current_apps_len = len(apps)
            layout = await self.dump_layout_to_json()
            app_list = utils.find_json_value_by_prev_path(
                layout, utils.find_json_value_as_path(layout, "List")[0], 2
            )
            app_paths = utils.find_json_value_as_path(app_list, "app_name")
            cur_apps = []
            apps_pos: dict[str, str] = {}
            for app_path in app_paths:
                try:
                    app = utils.find_json_value_by_prev_path(app_list, app_path)
                    app_pos = app["bounds"]
                    text = app["text"]
                except Exception:
                    continue
                if text in apps:
                    continue
                if utils.is_in_area(app_pos, bottom_bar):
                    logger.debug(f"[{self.tag}] 跳过底部按钮 [{text}]")
                    continue
                apps.append(text)
                cur_apps.append(text)
                apps_pos[text] = app_pos
                logger.debug(f"[{self.tag}] [{text}] [{app_pos}]")

            for _ in range(2):
                await self.device.simple_roll_down(0.5, 0.175, 10)
            if current_apps_len == len(apps):
                break
            # break
        await self.go_back(layout)

global_var: defaultdict[str, StorageValue] = defaultdict(lambda: StorageValue())
hilog_processes: dict[str, hdc.HilogProcess] = {}
skip_app_check = False
gallery_base_url = "https://hmos.txit.top/api"
fast_pull = False
skip_app_categories = False
skip_categories = []
ping = 15
keep_open_on_error = False
no_submit = False
# pulled_apps: defaultdict[str, list[str]] = defaultdict(list)
# pull_res: defaultdict[str, PullResult] = defaultdict(lambda: PullResult())
repeated_apps: bool = False
loop: Loop = Loop(0)
all_pulled_apps: defaultdict[str, set[str]] = defaultdict(set)

def parse_args(
    args: argparse.Namespace
):
    global \
        skip_app_check, \
        gallery_base_url, \
        fast_pull, \
        skip_app_categories, \
        skip_categories, \
        ping, \
        keep_open_on_error, \
        no_submit, \
        loop

    skip_app_check = args.skip_apps_check
    gallery_base_url = args.gallery_api
    fast_pull = args.fast_pull if hasattr(args, "fast_pull") else False
    skip_app_categories = args.skip_app_categories
    skip_categories = args.skip_categories
    ping = args.ping
    keep_open_on_error = getattr(args, "keep_open_on_error", False)
    no_submit = getattr(args, "no_submit", False)
    loop = Loop(max=args.loop, loop_wait=parse_time_units(args.loop_wait))

async def init(
    args: argparse.Namespace
):
    parse_args(args)

    if no_submit:
        logger.info("No Submit 模式已开启，跳过 Gallery API 初始化")
        return

    all_data_api = args.all_data_api
    all_data_api_key = args.all_data_api_key
    all_data_url = None
    if all_data_api is not None and all_data_api_key is not None:
        logger.info(f"使用 All Data API [{all_data_api}]")
        all_data_url = gallery.AllDataUrl(
            url=all_data_api,
            client_id=all_data_api_key
        )

    await gallery.init_gallery(gallery_base_url, all_data_url)

def loop_main(func: Callable[[], Coroutine]):
    async def inner_main(*args, **kwargs):
        global loop
        while loop.current < loop.max:
            start = runtime.perf_counter_ns()
            logger.info(f"开始第 [{loop.current + 1}] 轮")
            try:
                await func(*args, **kwargs)
            except Exception:
                logger.traceback("发生错误：")
                logger.error("正在重试...")
            finally:
                end = runtime.perf_counter_ns()
                logger.info(f"第 [{loop.current + 1}] 轮结束，耗时 [{format_count_time(end - start)}]")
            loop.current += 1
            logger.info(f"第 [{loop.current}] 轮结束")
            if loop.current < loop.max:
                logger.info(f"下一轮开始时间：[{datetime.datetime.now() + datetime.timedelta(seconds=loop.loop_wait)}]")
                await anyio.sleep(loop.loop_wait)
                
    return inner_main


async def start_app(device: AppGalleryCommonDevice):
    logger.info(f"[{device.tag}] 正在关闭 [AppGallery]")
    await device.close_app(APPGALLERY_PKG)
    await anyio.sleep(1)
    logger.info(f"[{device.tag}] 正在开启 [AppGallery]")
    await device.open_app(APPGALLERY_PKG, APPGALLERY_ABILITY)
    await anyio.sleep(3)
    logger.success(f"[{device.tag}] [AppGallery] 启动！")


def _find_tab_bounds(layout: Any, *candidates: str) -> Optional[str]:
    for candidate in candidates:
        bounds = utils.find_clickable_bounds_by_value(layout, candidate)
        if bounds is not None:
            return bounds
    return None


def _find_category_bounds(layout: Any) -> Optional[str]:
    return _find_tab_bounds(
        layout,
        "分类",
        "Paf_Lantern_Button_Index_1",
        "Paf_Lantern_Text_1",
        "Paf_Lantern_Normal_Image_1",
        "Paf_Lantern_Select_Image_1",
    )


async def _wait_for_bounds(
    device: AppGalleryCommonDevice,
    finder: Callable[[Any], Optional[str]],
    description: str,
    timeout: float = 4.0,
    interval: float = 0.35,
) -> str:
    deadline = runtime.perf_counter() + timeout
    while True:
        layout = await device.dump_layout_to_json()
        bounds = finder(layout)
        if bounds is not None:
            return bounds
        if runtime.perf_counter() >= deadline:
            break
        await anyio.sleep(interval)
    raise RuntimeError(f"[{device.tag}] 等待 [{description}] 超时，当前页面可能仍在切换或布局已变化")

async def go_app_page(device: AppGalleryCommonDevice):
    if global_var[device.sn].tab_app_btn is None:
        index_layout = await device.dump_layout_to_json()
        global_var[device.sn].tab_app_btn = _find_tab_bounds(
            index_layout,
            "应用",
            "BadgeImage.sys.symbol.bag_fill",
        )
        if global_var[device.sn].tab_app_btn is None:
            raise RuntimeError(f"[{device.tag}] 未找到 [应用] 页签，当前前台可能不是 AppGallery")
    btn = global_var[device.sn].tab_app_btn
    assert btn is not None
    logger.debug(f"[{device.tag}] 应用按钮位置 [{btn}]")
    await device.click_by_bounds(btn)


async def go_categories_page(device: AppGalleryCommonDevice):
    btn = await _wait_for_bounds(device, _find_category_bounds, "分类入口")
    logger.debug(f"[{device.tag}] 分类按钮位置 [{btn}]")
    await device.click_by_bounds(btn)


async def go_game_page(device: AppGalleryCommonDevice):
    if global_var[device.sn].tab_game_btn is None:
        index_layout = await device.dump_layout_to_json()
        global_var[device.sn].tab_game_btn = _find_tab_bounds(
            index_layout,
            "游戏",
            "BadgeImage.sys.symbol.game_fill",
        )
        if global_var[device.sn].tab_game_btn is None:
            raise RuntimeError(f"[{device.tag}] 未找到 [游戏] 页签，当前前台可能不是 AppGallery")
    btn = global_var[device.sn].tab_game_btn
    assert btn is not None
    logger.debug(f"[{device.tag}] 游戏按钮位置 [{btn}]")
    await device.click_by_bounds(btn, 1.75)


def device_main(device_class: Type[AppGalleryCommonDevice]):
    async def inner_main(func: Callable[[AppGalleryCommonDevice], Coroutine]):
        async def inner_device_main(device: hdc.Device):
            logger.info(f"[{device.tag}] 设备类型 [{device.device_type}]")
            global_var[device.sn].phone = device.device_type == "phone"
        
            app_gallery_info = await device.get_app_info(APPGALLERY_PKG)
            if app_gallery_info is not None:
                logger.info(
                    f"[{device.tag}] 应用商店版本 [{app_gallery_info.version_name} ({app_gallery_info.version_code})] 更新时间 [{app_gallery_info.update_time}]"
                )
                global_var[device.sn].app_info_version = app_gallery_info.version_code
            wrapper_device = device_class(device)
            start_time = runtime.perf_counter_ns()
            failed = False
            try:
                await func(wrapper_device)
            except Exception:
                failed = True
                logger.traceback(f"[{device.tag}] 发生错误")
            finally:
                if failed and keep_open_on_error:
                    logger.info(f"[{device.tag}] 调试模式已开启，保留 [AppGallery] 现场")
                else:
                    await device.close_app(APPGALLERY_PKG)
            end_time = runtime.perf_counter_ns()
            logger.success(
                f"[{device.tag}] 耗时 [{format_count_time(end_time - start_time)}] 共 [{wrapper_device.total_pull_res.total}] 个应用，新增 [{wrapper_device.total_pull_res.new}] 个应用"
            )
            no_repeated_apps = set(wrapper_device.all_pulled_apps)
            repeated_apps = set(
                [app for app in wrapper_device.all_pulled_apps if wrapper_device.all_pulled_apps.count(app) > 1]
            )
            logger.info(f"[{device.tag}] 共 [{len(no_repeated_apps)}] 个无重复应用")
            logger.info(f"[{device.tag}] 共 [{len(repeated_apps)}] 个有重复应用")
            # no_repeated_apps = list(no_repeated_apps)
            # repeated_apps = [app for app in pulled_apps if no_repeated_apps.count(app) > 1]
            display_repeated_apps = list(
                map(
                    lambda app: f"[{app}] [{wrapper_device.all_pulled_apps.count(app)}]",
                    sorted(repeated_apps),
                )
            )
            logger.info(f"[{device.tag}] 重复应用:")
            for i in range(0, len(display_repeated_apps), 5):
                logger.info(" ".join(display_repeated_apps[i : i + 5]))

        devices = await hdc.get_devices()
        for device in devices:
            logger.info(f"设备信息 [{device.tag}]")

        logger.info(
            f"一共有 [{len(devices)}] 台设备，其中有 [{len([d for d in devices if d.connection_type == 'tcp'])}] 是无线连接的"
        )

        async with anyio.create_task_group() as tg:
            for device in devices:
                tg.start_soon(inner_device_main, device)
        return func
    return inner_main

async def get_not_exists_apps(apps: list[str]) -> list[str]:
    if skip_app_check:
        return apps
    # return apps
    result = await gallery.get_gallery().search_app_names_exists(*apps)
    not_exists_apps = []
    for app, exists in result.items():
        if (repeated_apps and exists and app not in all_pulled_apps()) or (
            exists or app in all_pulled_apps()
        ):
            continue
        not_exists_apps.append(app)

    return not_exists_apps

