from collections import defaultdict
from dataclasses import dataclass
import datetime
import re
from typing import Optional
import anyio
from src import hdc, utils, hmgallery as gallery
from src.logger import logger
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


@dataclass
class PullResult:
    total: int = 0
    new: int = 0

    def add(self, val: "PullResult"):
        self.total += val.total
        self.new += val.new


APPGALLERY_PKG = "com.huawei.hmsapp.appgallery"
APPGALLERY_ABILITY = "MainAbility"
FUCKOFF_APPGALLERY_UPDATE = datetime.datetime.fromtimestamp(1767627470.362)
FUCKOFF_APPGALLERY_VERSION_CODE: int = 1460801300
FUCKOFF_SUB_CHUNKS = [re.compile("新鲜(应用|游戏)"), re.compile("时下畅销(应用|游戏)")]
global_var: defaultdict[str, StorageValue] = defaultdict(lambda: StorageValue())
hilog_processes: dict[str, hdc.HilogProcess] = {}
skip_app_check = False
gallery_base_url = "https://hmos.txit.top/api"
fast_pull = False
skip_app_categories = False
skip_categories = []
ping = 15
pulled_apps: defaultdict[str, list[str]] = defaultdict(list)
pull_res: defaultdict[str, PullResult] = defaultdict(lambda: PullResult())
repeated_apps: bool = False


async def device_main(device: hdc.Device):
    start_time = runtime.perf_counter_ns()
    try:
        await inner_device_main(device)
    except:  # noqa: E722
        logger.traceback(f"[{device.tag}] 发生错误")
    finally:
        await device.close_app(APPGALLERY_PKG)
    end_time = runtime.perf_counter_ns()
    logger.success(
        f"[{device.tag}] 耗时 [{format_count_time(end_time - start_time)}] 共 [{pull_res[device.sn].total}] 个应用，新增 [{pull_res[device.sn].new}] 个应用"
    )
    no_repeated_apps = set(pulled_apps[device.sn])
    repeated_apps = set(
        [app for app in pulled_apps if pulled_apps[device.sn].count(app) > 1]
    )
    logger.info(f"[{device.tag}] 共 [{len(no_repeated_apps)}] 个无重复应用")
    logger.info(f"[{device.tag}] 共 [{len(repeated_apps)}] 个有重复应用")
    # no_repeated_apps = list(no_repeated_apps)
    # repeated_apps = [app for app in pulled_apps if no_repeated_apps.count(app) > 1]
    display_repeated_apps = list(
        map(
            lambda app: f"[{app}] [{pulled_apps[device.sn].count(app)}]",
            sorted(repeated_apps),
        )
    )
    logger.info(f"[{device.tag}] 重复应用:")
    for i in range(0, len(display_repeated_apps), 5):
        logger.info(" ".join(display_repeated_apps[i : i + 5]))


async def inner_device_main(device: hdc.Device):
    logger.info(f"[{device.tag}] 设备类型 [{device.device_type}]")
    global_var[device.sn].phone = device.device_type == "phone"

    app_gallery_info = await device.get_app_info(APPGALLERY_PKG)
    if app_gallery_info is not None:
        logger.info(
            f"[{device.tag}] 应用商店版本 [{app_gallery_info.version_name} ({app_gallery_info.version_code})] 更新时间 [{app_gallery_info.update_time}]"
        )
        global_var[device.sn].app_info_version = app_gallery_info.version_code

    async with hdc.HilogProcess(
        device.device_id, "-e", "dashboard_shared", "-T", "JSAPP"
    ) as p:
        hilog_processes[device.sn] = p
        if fast_pull:
            await start_pull_apps(device)
        else:
            await start_app(device)
            if not skip_app_categories:
                await go_app_page(device)
                await go_categories_page(device)
                await pull_categories(device)
            else:
                logger.info("[{device.tag}] 跳过应用分类")
            await go_game_page(device)
            await go_categories_page(device)
            await pull_categories(device)


async def go_app_page(device: hdc.Device):
    if global_var[device.sn].tab_app_btn is None:
        index_layout = await device.dump_layout_to_json()
        global_var[device.sn].tab_app_btn = utils.find_json_value_by_prev_path(
            index_layout,
            utils.find_json_value_as_path(
                index_layout, "BadgeImage.sys.symbol.bag_fill"
            )[0],
        )["bounds"]
    btn = global_var[device.sn].tab_app_btn
    assert btn is not None
    logger.debug(f"[{device.tag}] 应用按钮位置 [{btn}]")
    await device.click_by_bounds(btn)


async def go_categories_page(device: hdc.Device):
    layout = await device.dump_layout_to_json()
    paths = utils.regex_json_value_as_path(
        layout, re.compile("^Paf_Lantern_(?:Select_|Normal_)?Image(?:_1)?$")
    )
    btn = utils.find_json_value_by_prev_path(
        layout, paths[0] if (len(paths) // 2) == 1 else paths[2]
    )["bounds"]
    await device.click_by_bounds(btn)


async def go_game_page(device: hdc.Device):
    if global_var[device.sn].tab_game_btn is None:
        index_layout = await device.dump_layout_to_json()
        global_var[device.sn].tab_game_btn = utils.find_json_value_by_prev_path(
            index_layout,
            utils.find_json_value_as_path(
                index_layout, "BadgeImage.sys.symbol.game_fill"
            )[0],
        )["bounds"]
    btn = global_var[device.sn].tab_game_btn
    assert btn is not None
    logger.debug(f"[{device.tag}] 游戏按钮位置 [{btn}]")
    await device.click_by_bounds(btn, 1.75)


async def pull_categories(device: hdc.Device):
    pulled_categories = []
    # idx = 1 if global_var[device.sn].phone else 2
    idx = -1 if global_var[device.sn].phone else 2
    while 1:
        current_categories_len = len(pulled_categories)

        layout = await device.dump_layout_to_json()
        layout = utils.find_json_value_by_prev_path(
            layout, utils.find_json_value_as_path(layout, "List")[idx], 2
        )
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
            logger.debug(f"[{device.tag}] [{text}] [{btn_pos}]")

            if text in skip_categories:
                continue

            await device.click_by_bounds(btn_pos, 1.75)
            await anyio.sleep(1 + ping * 0.05)
            logger.info(f"[{device.tag}] 正在拉取分类 [{text}]...")
            # await start_pull_apps(device, text)
            await pull_chunk_in_category(device, text)
        if current_categories_len == len(pulled_categories):
            break
        await device.simple_roll_down(0.5, 0.2, 0.72)


async def pull_chunk_in_category(device: hdc.Device, category: str):
    # 因为沟槽的华为更新了应用市场，所以现在需要先点进去分类，然后点进去子分类，最后再点进去应用
    exit_btn = None
    if global_var[device.sn].app_info_version < FUCKOFF_APPGALLERY_VERSION_CODE:
        await start_pull_apps(device, category)
    else:
        clicked_chunks = []
        retries = 0
        while len(clicked_chunks) < len(FUCKOFF_SUB_CHUNKS):
            current_chunks = len(clicked_chunks)
            layout = await device.dump_layout_to_json()
            if exit_btn is None:
                exit_btn = utils.find_json_value_by_prev_path(
                    layout, utils.find_json_value_as_path(layout, "BackButton")[0]
                )["bounds"]
            for chunk in FUCKOFF_SUB_CHUNKS:
                # if chunk in clicked_chunks:
                #     continue
                chunk_paths = utils.find_json_value_as_path(layout, chunk)
                if len(chunk_paths) == 0:
                    continue
                chunk_path = chunk_paths[0]
                match_chunk = utils.find_json_value_by_prev_path(layout, chunk_path)["text"]
                logger.info(f"[{device.tag}] 正在拉取分类 [{category}] 的 [{match_chunk}]...")
                await device.click_by_bounds(
                    utils.find_json_value_by_prev_path(layout, chunk_path)["bounds"],
                    1.75,
                )
                await anyio.sleep(1 + ping * 0.05)
                await start_pull_apps(device, f"{category} - {match_chunk}")
                clicked_chunks.append(match_chunk)
                # roll
            if current_chunks == len(clicked_chunks):
                if retries >= 3:
                    display_chunks = ", ".join(map(lambda x: f"[{x}]", clicked_chunks))
                    logger.warning(f"[{device.tag}] [{category}] 怎么只有 {display_chunks} 呢？")
                    break
                retries += 1
            await device.simple_roll_down(0.5, 0.2, 0.72)
    if exit_btn is not None:
        await device.click_by_bounds(exit_btn, 1.75)


async def start_pull_apps(device: hdc.Device, category: Optional[str] = None):
    # logger.info('正在开始拉取应用...')
    apps = []
    new_apps = []
    exit_btn = None
    start_time = runtime.perf_counter_ns()
    bottom_bar = await device.get_bottom_bar()
    while 1:
        current_apps_len = len(apps)
        layout = await device.dump_layout_to_json()
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
                logger.debug(f"[{device.tag}] 跳过底部按钮 [{text}]")
                continue
            apps.append(text)
            cur_apps.append(text)
            apps_pos[text] = app_pos
            logger.debug(f"[{device.tag}] [{text}] [{app_pos}]")

        pending_new_apps = await get_not_exists_apps(cur_apps)
        for app in pending_new_apps:
            logger.success(f"[{device.tag}] 发现新应用 [{app}]")
            await device.click_by_bounds(apps_pos[app], 1 + ping * 0.05)
            # detail
            await share_app(device, app)
            new_apps.append(app)

        await device.simple_roll_down(0.5, 0.175, 0.8)
        if current_apps_len == len(apps):
            break
        # break
    end_time = runtime.perf_counter_ns()
    elapsed_time = end_time - start_time
    avg_apps = elapsed_time / len(apps) if len(apps) > 0 else 0
    avg_new_apps = elapsed_time / len(new_apps) if len(new_apps) > 0 else 0
    display_category = f"[{category}] " if category else ""
    logger.info(
        f"[{device.tag}] {display_category}拉取应用完成, 共 [{len(apps)}] 个应用，新应用 [{len(new_apps)}] 个，耗时 [{format_count_time(elapsed_time)}] 平均 [{format_count_time(avg_apps)}/个] 新应用平均 [{format_count_time(avg_new_apps)}/个]"
    )
    exit_btn = utils.find_json_value_by_prev_path(
        layout, utils.find_json_value_as_path(layout, "BackButton")[0]
    )["bounds"]
    await device.click_by_bounds(exit_btn)
    pulled_apps[device.sn].extend(apps)
    pull_res[device.sn].add(PullResult(total=len(apps), new=len(new_apps)))


async def share_app(device: hdc.Device, app_name: str):
    if (
        global_var[device.sn].app_exit_btn is None
        or global_var[device.sn].app_share_btn is None
    ):
        layout = await device.dump_layout_to_json()

        # titlebar -> button
        titlebar = utils.find_json_value_by_prev_path(
            layout, utils.find_json_value_as_path(layout, "TitleBar")[0], 2
        )
        global_var[device.sn].app_share_btn = utils.find_json_value_by_prev_path(
            titlebar, utils.find_json_value_as_path(titlebar, "Button")[0]
        )["bounds"]
        global_var[device.sn].app_exit_btn = utils.find_json_value_by_prev_path(
            titlebar, utils.find_json_value_as_path(titlebar, "BackButton")[0]
        )["bounds"]

    share_btn = global_var[device.sn].app_share_btn
    assert share_btn is not None
    await device.click_by_bounds(share_btn, 1)

    btns = await find_and_click_share_app_btn(device)
    for btn in btns:
        await device.click_by_bounds(btn, 1)

    # share_layout = find_json_value_by_prev_path(share_layout, find_json_value_as_path(share_layout, "List")[0], 2)
    res = await find_app_link_in_logs(device)
    assert res is not None
    pkg = utils.parse_input_split_links_pkgs_and_app_ids(res).pkgs[-1]
    logger.success(f"[{device.tag}] [{app_name}] [{pkg}]")
    await anyio.sleep(0.95 + (ping * 0.05))

    exit_btn = global_var[device.sn].app_exit_btn
    assert exit_btn is not None
    await device.click_by_bounds(exit_btn)


async def find_app_link_in_logs(device: hdc.Device):
    hilog_process = hilog_processes[device.sn]
    while line := await hilog_process.readline():
        if "dashboard_shared" in line:
            return line
    return None


async def find_and_click_share_app_btn(device: hdc.Device) -> list[str]:
    if (
        app_direct_share_to_gallery_btn := global_var[
            device.sn
        ].app_direct_share_to_gallery_btn
    ) is not None:
        return [app_direct_share_to_gallery_btn]
    if (
        app_share_with_gallery_btn := global_var[device.sn].app_share_with_gallery_btn
    ) is not None and (
        app_share_to_gallery_btn := global_var[device.sn].app_share_to_gallery_btn
    ) is not None:
        return [app_share_with_gallery_btn, app_share_to_gallery_btn]
    if global_var[device.sn].app_direct_share_to_gallery_btn is None:
        share_layout = await device.dump_layout_to_json()
        final_share_path = utils.find_json_value_as_path(
            share_layout, "按已有信息投稿到看板"
        )
        if len(final_share_path) != 0:
            global_var[
                device.sn
            ].app_direct_share_to_gallery_btn = utils.find_json_value_by_prev_path(
                share_layout, final_share_path[0]
            )["bounds"]
            assert (
                app_direct_share_to_gallery_btn := global_var[
                    device.sn
                ].app_direct_share_to_gallery_btn
            ) is not None
            return [app_direct_share_to_gallery_btn]
        else:
            final_share_path = utils.find_json_value_as_path(share_layout, "应用看板")
            global_var[
                device.sn
            ].app_share_with_gallery_btn = utils.find_json_value_by_prev_path(
                share_layout, final_share_path[0]
            )["bounds"]
            assert (
                app_share_with_gallery_btn := global_var[
                    device.sn
                ].app_share_with_gallery_btn
            ) is not None
            await device.click_by_bounds(app_share_with_gallery_btn)

            app_view_layout = await device.dump_layout_to_json()
            global_var[
                device.sn
            ].app_share_to_gallery_btn = utils.find_json_value_by_prev_path(
                app_view_layout,
                utils.find_json_value_as_path(app_view_layout, "按已有信息投稿到看板")[
                    0
                ],
            )["bounds"]
            assert (
                app_share_to_gallery_btn := global_var[
                    device.sn
                ].app_share_to_gallery_btn
            ) is not None
            return [app_share_with_gallery_btn, app_share_to_gallery_btn]

    return []


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

def all_pulled_apps():
    res: set[str] = set()
    for apps in list(pulled_apps.values()):
        res = res.union(set(apps))
    return res



async def start_app(device: hdc.Device):
    logger.info(f"[{device.tag}] 正在关闭 [AppGallery]")
    await device.close_app(APPGALLERY_PKG)
    await anyio.sleep(1)
    logger.info(f"[{device.tag}] 正在开启 [AppGallery]")
    await device.open_app(APPGALLERY_PKG, APPGALLERY_ABILITY)
    await anyio.sleep(3)
    logger.success(f"[{device.tag}] [AppGallery] 启动！")


async def main(args):
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

    logger.info("AppGallery Pull Ciallo～ (∠・ω< )⌒★")
    logger.info(f"App Gallery API [{gallery_base_url}]")
    if skip_app_check:
        logger.info("跳过应用检查")
    gallery.init_gallery(gallery_base_url)
    devices = await hdc.get_devices()
    for device in devices:
        logger.info(f"设备信息 [{device.tag}]")

    logger.info(
        f"一共有 [{len(devices)}] 台设备，其中有 [{len([d for d in devices if d.connection_type == 'tcp'])}] 是无线连接的"
    )

    async with anyio.create_task_group() as tg:
        for device in devices:
            tg.start_soon(device_main, device)

    logger.info("AppGallery Pull 结束")
