import argparse
import asyncio
from dataclasses import dataclass
from typing import Optional
from src import hdc, hmgallery as gallery, utils
from src.logger import logger
from src.utils import find_json_value_as_path, find_json_value_by_prev_path

@dataclass
class StorageValue:
    tab_app_btn: Optional[str] = None
    app_exit_btn: Optional[str] = None
    app_share_btn: Optional[str] = None
    app_share_with_gallery_btn: Optional[str] = None
    app_share_to_gallery_btn: Optional[str] = None

global_var = StorageValue()
hilog_process: hdc.HilogProcess = None # type: ignore
parser = argparse.ArgumentParser()
parser.add_argument('--skip-apps-check', '-s', action='store_true', help='Skip App Check', default=False)
parser.add_argument('--gallery-api', default="https://hmos.txit.top/api")
parser.add_argument('--fast-pull', action='store_true', help='Fast Pull', default=False)
parser.add_argument('--skip-app-categories', '-c', action='store_true', help='Skip App Categories', default=False)
parser.add_argument('--skip-categories', '-k', help='Skip Categories', type=str, nargs="+", default=[])
args = parser.parse_known_args()[0]
skip_app_check = args.skip_apps_check
gallery_base_url = args.gallery_api
fast_pull = args.fast_pull
skip_app_categories = args.skip_app_categories
skip_categories = args.skip_categories

async def main():
    device_type = await hdc.get_device_type() 
    logger.info(f'当前设备类型: [{device_type}]')
    logger.info(f'App Gallery API: [{gallery_base_url}]')
    if skip_app_check:
        logger.info('跳过应用检查')
    gallery.init_gallery(gallery_base_url)
    async with hdc.HilogProcess("-e", "dashboard_shared", "-T", "JSAPP") as p:
        global hilog_process
        hilog_process = p
        if fast_pull:
            await start_pull_apps()
            return
        await open_gallery_app()
        if not skip_app_categories:
            await go_app_page()
            await go_categories_page()
            await pull_categories()
        else:
            logger.info('跳过应用分类')
        await go_game_page()
        await go_categories_page()
        await pull_categories()

async def open_gallery_app():
    logger.info('正在打开 [华为应用市场]...')
    await hdc.shell('aa force-stop com.huawei.hmsapp.appgallery')
    await hdc.shell('aa start -a MainAbility -b com.huawei.hmsapp.appgallery')
    logger.success('打开 [华为应用市场] 成功！')
    await asyncio.sleep(3)

async def go_app_page():
    if global_var.tab_app_btn is None:
        index_layout = await hdc.dump_layout_to_json()
        global_var.tab_app_btn = find_json_value_by_prev_path(index_layout, find_json_value_as_path(index_layout, "BadgeImage.sys.symbol.bag_fill")[0])['bounds']
    btn = global_var.tab_app_btn
    assert btn is not None
    logger.debug(f'应用按钮位置: [{btn}]')
    await hdc.click_by_bounds(btn)

async def go_game_page():
    if global_var.tab_app_btn is None:
        index_layout = await hdc.dump_layout_to_json()
        global_var.tab_app_btn = find_json_value_by_prev_path(index_layout, find_json_value_as_path(index_layout, "BadgeImage.sys.symbol.game_fill")[0])['bounds']
    btn = global_var.tab_app_btn
    assert btn is not None
    logger.debug(f'应用按钮位置: [{btn}]')
    await hdc.click_by_bounds(btn)

async def go_categories_page():
    layout = await hdc.dump_layout_to_json()
    btn = find_json_value_by_prev_path(layout, find_json_value_as_path(layout, "Paf_Lantern_Image")[2])['bounds']
    await hdc.click_by_bounds(btn)

    ...

async def pull_categories():
    pulled_categories = []
    while 1:
        current_categories_len = len(pulled_categories)

        layout = await hdc.dump_layout_to_json()
        layout = find_json_value_by_prev_path(layout, find_json_value_as_path(layout, "List")[1], 2)
        # and then fuck to find btn
        btns = find_json_value_as_path(layout, "Button")
        for btn_path in btns:
            try:
                btn = find_json_value_by_prev_path(layout, btn_path)
                btn_pos = btn['bounds']
                txt = find_json_value_by_prev_path(layout, btn_path, 2)
                text = find_json_value_by_prev_path(txt, find_json_value_as_path(txt, "Text")[0])['text']
            except Exception:
                continue
            if text in pulled_categories:
                continue
            pulled_categories.append(text)
            logger.debug(f'[{text}] [{btn_pos}]')

            if text in skip_categories:
                continue

            await hdc.click_by_bounds(btn_pos)
            await asyncio.sleep(1)
            await start_pull_apps()

        if current_categories_len == len(pulled_categories):
            break
        await hdc.simple_roll_down(0.5, 0.2, 0.72)
    # find btns and then roll down


async def get_not_exists_apps(
    apps: list[str]
) -> list[str]:
    if skip_app_check:
        return apps
    # return apps
    result = await gallery.get_gallery().search_app_names_exists(*apps)
    not_exists_apps = []
    for app, exists in result.items():
        if exists:
            continue
        not_exists_apps.append(app)
    return not_exists_apps

async def start_pull_apps():
    logger.info('正在开始拉取应用...')
    apps = []
    exit_btn = None
    while 10:
        current_apps_len = len(apps)
        layout = await hdc.dump_layout_to_json()
        app_list = find_json_value_by_prev_path(layout, find_json_value_as_path(layout, "List")[0], 2)
        app_paths = find_json_value_as_path(app_list, "app_name")
        cur_apps = []
        apps_pos: dict[str, str] = {}
        for app_path in app_paths:
            try:
                app = find_json_value_by_prev_path(app_list, app_path)
                app_pos = app['bounds']
                text = app['text']
            except Exception:
                continue
            if text in apps:
                continue
            apps.append(text)
            cur_apps.append(text)
            apps_pos[text] = app_pos
            logger.debug(f'[{text}] [{app_pos}]')

        new_apps = await get_not_exists_apps(cur_apps)
        for app in new_apps:
            logger.success(f'发现新应用 [{app}]')
            await hdc.click_by_bounds(apps_pos[app], 0.85)
            # detail 
            await share_app(app)


        await hdc.simple_roll_down(0.5, 0.175, 0.8)
        if current_apps_len == len(apps):
            break
        # break

    exit_btn = find_json_value_by_prev_path(layout, find_json_value_as_path(layout, "BackButton")[0])['bounds']
    await hdc.click_by_bounds(exit_btn)

async def find_app_link_in_logs():
    global hilog_process
    assert hilog_process is not None
    while line := await hilog_process.readline():
        if "dashboard_shared" in line:
            return line
    return None

async def share_app(
    app_name: str
):
    if global_var.app_exit_btn is None or global_var.app_share_btn is None:
        layout = await hdc.dump_layout_to_json()

        # titlebar -> button
        titlebar = find_json_value_by_prev_path(layout, find_json_value_as_path(layout, "TitleBar")[0], 2)
        global_var.app_share_btn = find_json_value_by_prev_path(titlebar, find_json_value_as_path(titlebar, "Button")[0])['bounds']
        global_var.app_exit_btn = find_json_value_by_prev_path(titlebar, find_json_value_as_path(titlebar, "BackButton")[0])['bounds']

    share_btn = global_var.app_share_btn
    assert share_btn is not None
    await hdc.click_by_bounds(share_btn)

    # share layout

    if global_var.app_share_with_gallery_btn is None:
        share_layout = await hdc.dump_layout_to_json()
        # "应用看板"
        global_var.app_share_with_gallery_btn = find_json_value_by_prev_path(share_layout, find_json_value_as_path(share_layout, "应用看板")[0])['bounds']
    app_view_btn = global_var.app_share_with_gallery_btn
    assert app_view_btn is not None
    await hdc.click_by_bounds(app_view_btn)

    if global_var.app_share_to_gallery_btn is None:
        app_view_layout = await hdc.dump_layout_to_json()
        global_var.app_share_to_gallery_btn = find_json_value_by_prev_path(app_view_layout, find_json_value_as_path(app_view_layout, "按已有信息投稿到看板")[0])['bounds']
    share_app_btn = global_var.app_share_to_gallery_btn
    assert share_app_btn is not None
    await hdc.click_by_bounds(share_app_btn)
    # share_layout = find_json_value_by_prev_path(share_layout, find_json_value_as_path(share_layout, "List")[0], 2)
    res = await find_app_link_in_logs()
    assert res is not None
    pkg = utils.parse_input_split_links_pkgs_and_app_ids(res).pkgs[-1]
    logger.success(f'[{app_name}] [{pkg}]')
    await asyncio.sleep(0.65)


    exit_btn = global_var.app_exit_btn
    assert exit_btn is not None
    await hdc.click_by_bounds(exit_btn)
