from collections import defaultdict
from dataclasses import dataclass
import datetime
import re
from typing import Optional
import anyio
from src import hdc, utils, hmgallery as gallery
from src.logger import logger
from tianxiu2b2t.utils import runtime
from tianxiu2b2t.units import format_count_time, parse_time_units
from .common import go_categories_page, go_app_page, go_game_page
from core import common

class AppGalleryGalleryDevice(common.AppGalleryCommonDevice):
    def __init__(self, device: hdc.Device):
        super().__init__(device)
        self.process: hdc.HilogProcess = None # type: ignore
    
    def set_process(self, process: hdc.HilogProcess):
        self.process = process

@common.device_main(device_class=AppGalleryGalleryDevice)
async def device_main(device: common.AppGalleryCommonDevice):
    async with hdc.HilogProcess(
        device.device_id, "-e", "dashboard_shared", "-T", "JSAPP"
    ) as dashboard_process:
        device.set_process(dashboard_process)

        if common.fast_pull:
            await device.start_pull_apps()



async def inner_device_main(device: hdc.Device):
    async with hdc.HilogProcess(
        device.device_id, "-e", "dashboard_shared", "-T", "JSAPP"
    ) as dashboard_process, hdc.HilogProcess(
        device.device_id,
    ) as appgallery_hilog, anyio.create_task_group() as task_group:
        hilog_processes[device.sn] = HiLogProcesses(
            dashboard_process=dashboard_process, appgallery_hilog=appgallery_hilog
        )
        task_group.start_soon(pull_in_appgallery_logs, device)
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

async def pull_categories(device: hdc.Device):
    pulled_categories = []
    while 1:
        current_categories_len = len(pulled_categories)

        layout = await device.dump_layout_to_json()
        layout = common.find_categories_list(layout)
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

            if text in common.CATEGORY_PAGE_TABS:
                logger.debug(f"[{device.tag}] 跳过分类页导航 [{text}]")
                continue

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

async def get_new_ui(device: hdc.Device) -> bool:
    if (val := global_var[device.sn].is_new_ui) is not None:
        return val
    layout = await device.dump_layout_to_json()
    val = sum([len(utils.find_json_value_as_path(layout, chunk)) for chunk in FUCKOFF_SUB_CHUNKS]) != 0
    global_var[device.sn].is_new_ui = val
    return val

async def pull_chunk_in_category(device: hdc.Device, category: str):
    # 因为沟槽的华为更新了应用市场，所以现在需要先点进去分类，然后点进去子分类，最后再点进去应用
    # patch: 傻逼华为，妈的，为什么还要分设备的应用市场，草泥马的
    layout = await device.dump_layout_to_json()
    new_ui = await get_new_ui(device)
    # or global_var[device.sn].app_info_version < FUCKOFF_APPGALLERY_VERSION_CODE
    if not new_ui:
        await start_pull_apps(device, category)
    else:
        clicked_chunks = []
        retries = 0
        while len(clicked_chunks) < len(common.FUCKOFF_SUB_CHUNKS):
            current_chunks = len(clicked_chunks)
            layout = await device.dump_layout_to_json()
            for chunk in common.FUCKOFF_SUB_CHUNKS:
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
                await anyio.sleep(1 + common.ping * 0.05)
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
        await device.go_back(wait_for=1.75)


async def start_pull_apps(device: hdc.Device, category: Optional[str] = None):
    # logger.info('正在开始拉取应用...')
    apps = []
    new_apps = []
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
                await device.click_by_bounds(apps_pos[app], 1 + common.ping * 0.05)
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
    await device.go_back(layout)
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
        global_var[device.sn].app_exit_btn = device.find_back_bounds(titlebar)

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
    if exit_btn is not None:
        await device.click_by_bounds(exit_btn)
    else:
        await device.go_back(wait_for=0.75)


async def find_app_link_in_logs(device: hdc.Device):
    hilog_process = hilog_processes[device.sn]
    while line := await hilog_process.dashboard_process.readline():
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

async def submit_app(
    device: hdc.Device,
    pkg: Optional[str],
    app_id: Optional[str]
):
    if pkg is None and app_id is None:
        return
    commit_app = pkg or app_id
    assert commit_app is not None
    logger.info(f"[{device.tag}] 正在提交应用 [{commit_app}]")
    res = await gallery.get_gallery().submit_app(pkg, app_id, gallery.CommentInfo(
        user=username
    ))
    if not res:
        logger.error(f"[{device.tag}] 提交 [{commit_app}] 失败")
        return
    new_app = res["new_app"]
    if new_app:
        logger.success(f"[{device.tag}] 提交 [{commit_app}] 成功")
    else:
        logger.warning(f"[{device.tag}] 提交 [{commit_app}] 成功，应用已存在")

async def pull_in_appgallery_logs(
    device: hdc.Device,
):
    # pulled_pkgs: set[str] = set()
    # pulled_app_ids: set[str] = set()
    process = hilog_processes[device.sn].appgallery_hilog
    async with anyio.create_task_group() as tg:
        while line := await process.readline():
            res = hdc.parse_hilog_line(line)
            if res is None:
                continue
            if res.level != "E" or "GetSpecifiedDistributionType failed -n" not in res.log:
                continue
            parsed_res = utils.parse_input_split_links_pkgs_and_app_ids(res.log)
            for pkg in parsed_res.pkgs:
                if pkg in pulled_pkgs:
                    continue
                pulled_pkgs.add(pkg)
                # logger.info(f"[{device.tag}] 拉取应用 [{pkg}] 成功！")
                tg.start_soon(submit_app, device, pkg, None)

            for app_id in parsed_res.app_ids:
                if app_id in pulled_app_ids:
                    continue
                pulled_app_ids.add(app_id)
                tg.start_soon(submit_app, device, None, app_id)
                # logger.info(f"[{device.tag}] 拉取应用 [{app_id}] 成功！")

async def start_app(device: hdc.Device):
    logger.info(f"[{device.tag}] 正在关闭 [AppGallery]")
    await device.close_app(APPGALLERY_PKG)
    await anyio.sleep(1)
    logger.info(f"[{device.tag}] 正在开启 [AppGallery]")
    await device.open_app(APPGALLERY_PKG, APPGALLERY_ABILITY)
    await anyio.sleep(3)
    logger.success(f"[{device.tag}] [AppGallery] 启动！")


@common.loop_main
async def inner_main():
    await device_main

async def main(args):
    logger.info("AppGallery Pull 结束")
