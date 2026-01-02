import argparse
import asyncio
from dataclasses import dataclass
from typing import Optional
from . import hdc as test_hdc, old_hdc
from src import hmgallery as gallery, utils
from src.logger import logger
from src.utils import find_json_value_as_path, find_json_value_by_prev_path
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


@dataclass
class PullResult:
    total: int = 0
    new: int = 0

    def add(self, val: "PullResult"):
        self.total += val.total
        self.new += val.new


global_var = StorageValue()
hilog_process: old_hdc.HilogProcess = None  # type: ignore
skip_app_check = False
gallery_base_url = "https://hmos.txit.top/api"
fast_pull = False
skip_app_categories = False
skip_categories = []
ping = 5
pulled_apps: list[str] = []
pull_res = PullResult()


async def start():
    targets = await test_hdc.get_targets()
    for uid, device in targets.items():
        print(uid, device)
        print(await device.get_ping())
    device_type = await old_hdc.get_device_type()
    global_var.phone = device_type == "phone"
    logger.info("AppGallery Ciallo～ (∠・ω< )⌒★")
    logger.info(f"当前设备类型 [{device_type}]")
    logger.info(f"App Gallery API [{gallery_base_url}]")
    if skip_app_check:
        logger.info("跳过应用检查")

    gallery.init_gallery(gallery_base_url)
    await fuck_off_usb_connection_type() # 关闭USB连接UI

    async with old_hdc.HilogProcess("-e", "dashboard_shared", "-T", "JSAPP") as p:
        global hilog_process
        hilog_process = p
        if fast_pull:
            await start_pull_apps()
        else:
            await open_gallery_app()
            if not skip_app_categories:
                await go_app_page()
                await go_categories_page()
                await pull_categories()
            else:
                logger.info("跳过应用分类")
            await go_game_page()
            await go_categories_page()
            await pull_categories()


async def main(args: argparse.Namespace):
    global \
        skip_app_check, \
        gallery_base_url, \
        fast_pull, \
        skip_app_categories, \
        skip_categories, \
        ping

    skip_app_check = args.skip_apps_check
    gallery_base_url = args.gallery_api
    fast_pull = args.fast_pull
    skip_app_categories = args.skip_app_categories
    skip_categories = args.skip_categories
    ping = args.ping
    start_time = runtime.perf_counter_ns()
    try:
        await start()
    except Exception:
        logger.traceback()
        await kill_gallery_app()
    finally:
        end_time = runtime.perf_counter_ns()

    logger.success(
        f"耗时 [{format_count_time(end_time - start_time)}] 共 [{pull_res.total}] 个应用，新增 [{pull_res.new}] 个应用"
    )
    no_repeated_apps = set(pulled_apps)
    repeated_apps = set([app for app in pulled_apps if pulled_apps.count(app) > 1])
    logger.info(f"共 [{len(no_repeated_apps)}] 个无重复应用")
    logger.info(f"共 [{len(repeated_apps)}] 个有重复应用")
    # no_repeated_apps = list(no_repeated_apps)
    # repeated_apps = [app for app in pulled_apps if no_repeated_apps.count(app) > 1]
    display_repeated_apps = list(
        map(lambda app: f"[{app}] [{pulled_apps.count(app)}]", sorted(repeated_apps))
    )
    logger.info("重复应用:")
    for i in range(0, len(display_repeated_apps), 5):
        logger.info(" ".join(display_repeated_apps[i : i + 5]))


async def kill_gallery_app():
    await old_hdc.shell("aa", "force-stop", "com.huawei.hmsapp.appgallery")


async def open_gallery_app():
    logger.info("正在打开 [华为应用市场]...")
    await kill_gallery_app()
    await old_hdc.shell(
        "aa", "start", "-a", "MainAbility", "-b", "com.huawei.hmsapp.appgallery"
    )
    logger.success("打开 [华为应用市场] 成功！")
    await asyncio.sleep(3)

async def fuck_off_usb_connection_type():
    layout = await old_hdc.dump_layout_to_json()
    path = find_json_value_as_path(layout, "USB 连接方式")
    if not path:
        return
    await old_hdc.click_pos_by_scale(0.5, 0.8)


async def go_app_page():
    if global_var.tab_app_btn is None:
        index_layout = await old_hdc.dump_layout_to_json()
        global_var.tab_app_btn = find_json_value_by_prev_path(
            index_layout,
            find_json_value_as_path(index_layout, "BadgeImage.sys.symbol.bag_fill")[0],
        )["bounds"]
    btn = global_var.tab_app_btn
    assert btn is not None
    logger.debug(f"应用按钮位置 [{btn}]")
    await old_hdc.click_by_bounds(btn)


async def go_game_page():
    if global_var.tab_game_btn is None:
        index_layout = await old_hdc.dump_layout_to_json()
        global_var.tab_game_btn = find_json_value_by_prev_path(
            index_layout,
            find_json_value_as_path(index_layout, "BadgeImage.sys.symbol.game_fill")[0],
        )["bounds"]
    btn = global_var.tab_game_btn
    assert btn is not None
    logger.debug(f"游戏按钮位置 [{btn}]")
    await old_hdc.click_by_bounds(btn, 1.75)


async def go_categories_page():
    layout = await old_hdc.dump_layout_to_json()
    btn = find_json_value_by_prev_path(
        layout, find_json_value_as_path(layout, "Paf_Lantern_Image")[2]
    )["bounds"]
    await old_hdc.click_by_bounds(btn)


async def pull_categories():
    pulled_categories = []
    idx = 1 if global_var.phone else 2
    while 1:
        current_categories_len = len(pulled_categories)

        layout = await old_hdc.dump_layout_to_json()
        layout = find_json_value_by_prev_path(
            layout, find_json_value_as_path(layout, "List")[idx], 2
        )
        # and then fuck to find btn
        btns = find_json_value_as_path(layout, "Button")
        for btn_path in btns:
            try:
                btn = find_json_value_by_prev_path(layout, btn_path)
                btn_pos = btn["bounds"]
                txt = find_json_value_by_prev_path(layout, btn_path, 2)
                text = find_json_value_by_prev_path(
                    txt, find_json_value_as_path(txt, "Text")[0]
                )["text"]
            except Exception:
                continue
            if text in pulled_categories:
                continue
            pulled_categories.append(text)
            logger.debug(f"[{text}] [{btn_pos}]")

            if text in skip_categories:
                continue

            await old_hdc.click_by_bounds(btn_pos, 1.75)
            await asyncio.sleep(1 + ping * 0.05)
            logger.info(f"正在拉取分类 [{text}]...")
            await start_pull_apps(text)
        if current_categories_len == len(pulled_categories):
            break
        await old_hdc.simple_roll_down(0.5, 0.2, 0.72)


async def get_not_exists_apps(apps: list[str]) -> list[str]:
    if skip_app_check:
        return apps
    # return apps
    result = await gallery.get_gallery().search_app_names_exists(*apps)
    not_exists_apps = []
    for app, exists in result.items():
        if exists and app not in pulled_apps:
            continue
        not_exists_apps.append(app)
    
    # pulled_apps
    # for app in apps:
    #     if app in pulled_apps and app not in not_exists_apps:
    #         not_exists_apps.append(app)
    return not_exists_apps


async def start_pull_apps(category: Optional[str] = None):
    # logger.info('正在开始拉取应用...')
    apps = []
    new_apps = []
    exit_btn = None
    start_time = runtime.perf_counter_ns()
    while 1:
        current_apps_len = len(apps)
        layout = await old_hdc.dump_layout_to_json()
        app_list = find_json_value_by_prev_path(
            layout, find_json_value_as_path(layout, "List")[0], 2
        )
        app_paths = find_json_value_as_path(app_list, "app_name")
        cur_apps = []
        apps_pos: dict[str, str] = {}
        for app_path in app_paths:
            try:
                app = find_json_value_by_prev_path(app_list, app_path)
                app_pos = app["bounds"]
                text = app["text"]
            except Exception:
                continue
            if text in apps:
                continue
            apps.append(text)
            cur_apps.append(text)
            apps_pos[text] = app_pos
            logger.debug(f"[{text}] [{app_pos}]")

        pending_new_apps = await get_not_exists_apps(cur_apps)
        for app in pending_new_apps:
            logger.success(f"发现新应用 [{app}]")
            await old_hdc.click_by_bounds(apps_pos[app], 1 + ping * 0.05)
            # detail
            await share_app(app)
            new_apps.append(app)

        await old_hdc.simple_roll_down(0.5, 0.175, 0.8)
        if current_apps_len == len(apps):
            break
        # break
    end_time = runtime.perf_counter_ns()
    elapsed_time = end_time - start_time
    avg_apps = elapsed_time / len(apps) if len(apps) > 0 else 0
    avg_new_apps = elapsed_time / len(new_apps) if len(new_apps) > 0 else 0
    display_category = f"[{category}] " if category else ""
    logger.info(
        f"{display_category}拉取应用完成, 共 [{len(apps)}] 个应用，新应用 [{len(new_apps)}] 个，耗时 [{format_count_time(elapsed_time)}] 平均 [{format_count_time(avg_apps)}/个] 新应用平均 [{format_count_time(avg_new_apps)}/个]"
    )
    exit_btn = find_json_value_by_prev_path(
        layout, find_json_value_as_path(layout, "BackButton")[0]
    )["bounds"]
    await old_hdc.click_by_bounds(exit_btn)
    pulled_apps.extend(apps)
    pull_res.add(PullResult(total=len(apps), new=len(new_apps)))


async def find_app_link_in_logs():
    global hilog_process
    assert hilog_process is not None
    while line := await hilog_process.readline():
        if "dashboard_shared" in line:
            return line
    return None

async def find_and_click_share_app_btn() -> list[str]:
    if global_var.app_direct_share_to_gallery_btn is not None:
        return [global_var.app_direct_share_to_gallery_btn]
    if global_var.app_share_with_gallery_btn is not None and global_var.app_share_to_gallery_btn is not None:
        return [global_var.app_share_with_gallery_btn, global_var.app_share_to_gallery_btn]
    if global_var.app_direct_share_to_gallery_btn is None:
        share_layout = await old_hdc.dump_layout_to_json()
        final_share_path = find_json_value_as_path(share_layout, "按已有信息投稿到看板")
        if len(final_share_path) != 0:
            global_var.app_direct_share_to_gallery_btn = find_json_value_by_prev_path(
                share_layout, final_share_path[0]
            )["bounds"]
            assert global_var.app_direct_share_to_gallery_btn is not None
            return [global_var.app_direct_share_to_gallery_btn]
        else:
            final_share_path = find_json_value_as_path(share_layout, "应用看板")
            global_var.app_share_with_gallery_btn = find_json_value_by_prev_path(
                share_layout, final_share_path[0]
            )["bounds"]
            assert global_var.app_share_with_gallery_btn is not None
            await old_hdc.click_by_bounds(global_var.app_share_with_gallery_btn)
        
            app_view_layout = await old_hdc.dump_layout_to_json()
            global_var.app_share_to_gallery_btn = find_json_value_by_prev_path(
                app_view_layout,
                find_json_value_as_path(app_view_layout, "按已有信息投稿到看板")[0],
            )["bounds"]
            assert global_var.app_share_to_gallery_btn is not None
            return [global_var.app_share_with_gallery_btn, global_var.app_share_to_gallery_btn]
            
    return []
    

async def share_app(app_name: str):
    if global_var.app_exit_btn is None or global_var.app_share_btn is None:
        layout = await old_hdc.dump_layout_to_json()

        # titlebar -> button
        titlebar = find_json_value_by_prev_path(
            layout, find_json_value_as_path(layout, "TitleBar")[0], 2
        )
        global_var.app_share_btn = find_json_value_by_prev_path(
            titlebar, find_json_value_as_path(titlebar, "Button")[0]
        )["bounds"]
        global_var.app_exit_btn = find_json_value_by_prev_path(
            titlebar, find_json_value_as_path(titlebar, "BackButton")[0]
        )["bounds"]

    share_btn = global_var.app_share_btn
    assert share_btn is not None
    await old_hdc.click_by_bounds(share_btn, 1)

    btns = await find_and_click_share_app_btn()
    for btn in btns:
        await old_hdc.click_by_bounds(btn, 1)

    # share_layout = find_json_value_by_prev_path(share_layout, find_json_value_as_path(share_layout, "List")[0], 2)
    res = await find_app_link_in_logs()
    assert res is not None
    pkg = utils.parse_input_split_links_pkgs_and_app_ids(res).pkgs[-1]
    logger.success(f"[{app_name}] [{pkg}]")
    await asyncio.sleep(0.65)

    exit_btn = global_var.app_exit_btn
    assert exit_btn is not None
    await old_hdc.click_by_bounds(exit_btn)
