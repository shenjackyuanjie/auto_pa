import argparse
from dataclasses import dataclass
from typing import Optional
import anyio
import src.hdc as hdc
import src.utils as utils
import src.hmgallery as gallery
from src.logger import logger
from tianxiu2b2t.anyio.future import Future


argument = argparse.ArgumentParser()
argument.add_argument('--skip-categories', '--sc', type=str, nargs="+")
argument.add_argument('--fast-pull', '--fp', help="Fast pull app, not check devices in main page", action="store_true")
argument.add_argument('--gallery-api', '--ga', help="Use gallery api to pull app", default="https://hmos.txit.top/api")
argument.add_argument('--username', '-u', help="Submit app with this username", required=True)
argument.add_argument('--submit', '-s', help="In python, submit app to gallery", action="store_true")
argument.add_argument('--skip-check-apps', '--sca', help="Skip check apps in gallery", action="store_true")

@dataclass
class AppInCategory:
    name: str
    bounds: tuple[float, float, float, float]

main_layout_res = None
app_categories_res = None
app_categories_size = None
skip_categories = []
submit_username = ''
share_layout_res = None
share_with_gallery_view_page_res = None
app_detail_layout_res = None
submit_in_python = False
skip_check_apps = False

async def is_main_page():
    main_screen = await hdc.get_main_screen_size()
    res = await hdc.dump_layout_to_json()
    paths = utils.find_json_value_as_path(res, "tab_text")
    try:
        value = utils.find_json_value_by_prev_path(res, paths[0])
    except Exception:
        return False
    bounds = utils.parse_bounds(value['bounds'])
    phone_mode = await hdc.is_phone_mode()
    if phone_mode:     # 判断是不是在屏幕下方
        result = bounds[1] > main_screen[1] * 0.8
    else:
        result = bounds[0] < main_screen[0] * 0.2
    if result:
        global main_layout_res
        main_layout_res = res
    return result

async def wait_back_to_main_page():
    while not await is_main_page():
        logger.warning("请到 应用市场 主页之后开始操作")
        await anyio.sleep(5)

async def click_bottom_bar_app():
    global main_layout_res
    if main_layout_res is None:
        await wait_back_to_main_page()

    paths = utils.find_json_value_as_path(main_layout_res, "应用")
    value = utils.find_json_value_by_path(main_layout_res, paths[0][:-1])
    bounds = utils.parse_bounds(value['bounds'])
    await hdc.click_by_bounds(bounds)

async def is_app_page():
    res = await hdc.dump_layout_to_json()
    paths = utils.find_json_value_as_path(res, "分类")
    try:
        _ = utils.find_json_value_by_path(res, paths[0][:-1])
    except Exception:
        return False
    global app_categories_res
    app_categories_res = res
    return True

async def wait_for_app_page():
    while not await is_app_page():
        logger.warning("请到 应用市场 应用主页之后开始操作")
        await anyio.sleep(5)

async def is_app_categories_page():
    res = await hdc.dump_layout_to_json()
    paths = utils.find_json_value_as_path(res, "分类")
    try:
        value = utils.find_json_value_by_path(res, paths[0][:-1])
    except Exception:
        return False
    
    
    return value['backgroundColor'] == '#FFFFFFFF'

async def wait_for_app_categories_page():
    while not await is_app_categories_page():
        logger.warning("请到 应用市场 应用分类页之后开始操作")
        await anyio.sleep(5)

async def click_app_categories():
    await wait_for_app_page()
    if await is_app_categories_page():
        return
    paths = utils.find_json_value_as_path(app_categories_res, "分类")
    value = utils.find_json_value_by_path(app_categories_res, paths[0][:-1])
    bounds = utils.parse_bounds(value['bounds'])
    await hdc.click_pos((bounds[0] + bounds[2]) / 2, (bounds[1] + bounds[3]) / 2)

async def get_app_categories():
    await wait_for_app_categories_page()
    global app_categories_res
    if app_categories_res is None:
        await wait_for_app_categories_page()

async def pull_app_in_categories():
    await wait_for_app_categories_page()
    global app_categories_res
    if app_categories_res is None:
        await wait_for_app_categories_page()

    main_screen_size = await hdc.get_main_screen_size()
    
    clicked_categories: set[str] = set()
    stable_count = 0
    while stable_count < 2:
        current_clicked_categories = set()
        res = await hdc.dump_layout_to_json()
        list_items = utils.find_json_value_by_prev_path(res, utils.find_json_value_as_path(res, "List")[-1], 2)
        buttons = utils.list_json_value_by_prev_paths(list_items, utils.find_json_value_as_path(list_items, "Text"))
        if not buttons:
            continue
        for btn in buttons:
            text = btn['text']
            if text is None or text == '' or text in clicked_categories:
                continue
            bounds = utils.parse_bounds(btn['bounds'])
            if bounds[3] > main_screen_size[1] * 0.9:
                break
            clicked_categories.add(text)
            current_clicked_categories.add(text)
            if text in skip_categories:
                logger.warning(f"跳过分类 [{text}]")
                continue
            logger.info(f"点击分类 [{text}]")
            await hdc.click_by_bounds(bounds)

            await anyio.sleep(1.25)
            # start pull apps
            await next_pull_apps_in_categories()
            

        # roll down
        main_screen_size = await hdc.get_main_screen_size()
        await hdc.roll_to_y(main_screen_size[0] * 0.5, main_screen_size[1] * 0.2, main_screen_size[1] * 0.72)
        
        if not current_clicked_categories:
            stable_count += 1
            logger.debug(f"没有找到可点击的分类 [{stable_count}]")
            continue

        stable_count = 0

async def get_not_exists_apps(
    apps: list[str]
) -> list[str]:
    if skip_check_apps:
        return apps
    # return apps
    result = await gallery.get_gallery().search_app_names_exists(*apps)
    not_exists_apps = []
    for app, exists in result.items():
        if exists:
            continue
        not_exists_apps.append(app)
    return not_exists_apps

async def find_app_link_in_logs(
    result: Future
):
    hilog = await hdc.advanced_hilog(("-e", "dashboard_shared", "-T", "JSAPP"), ("-m", "1"), "dashboard_shared")
    print(hilog)
    result.set_result(hilog[-1])
    return hilog
    

async def click_app_in_category_and_share(
    app_name: str,
    bounds: tuple[int, int, int, int]
) -> Optional[str]:
    await hdc.click_by_bounds(bounds)

    global app_detail_layout_res, share_layout_res, share_with_gallery_view_page_res
    if app_detail_layout_res is None:
        app_detail_layout_res = await hdc.dump_layout_to_json()
    app_detail_layout = app_detail_layout_res

    back_btn = utils.find_json_value_by_prev_path(app_detail_layout, utils.find_json_value_as_path(app_detail_layout, "__NavdestinationField__BackButton__Back__")[0])['bounds']
    share_btn = utils.find_json_value_by_prev_path(app_detail_layout, utils.find_json_value_as_path(app_detail_layout, "detail_share_menu")[0])['bounds']
    if back_btn is None or share_btn is None:
        logger.error(f"[{app_name}] 没有找到返回按钮或分享按钮")
        return

    # await anyio.sleep(0.75) # wait for network pull


    await hdc.click_by_bounds(utils.parse_bounds(share_btn))

    if share_layout_res is None:
        share_layout_res = await hdc.dump_layout_to_json()
    share_layout = share_layout_res

    share_to_gallery_view = utils.find_json_value_by_prev_path(share_layout, utils.find_json_value_as_path(share_layout, "应用看板")[0])['bounds']
    await hdc.click_by_bounds(utils.parse_bounds(share_to_gallery_view))

    if share_with_gallery_view_page_res is None:
        share_with_gallery_view_page_res = await hdc.dump_layout_to_json()
    share_with_gallery_view_page = share_with_gallery_view_page_res

    gallery_view_btn = utils.find_json_value_by_prev_path(share_with_gallery_view_page, utils.find_json_value_as_path(share_with_gallery_view_page, "按已有信息投稿到看板")[0])['bounds']
    link_fut: Future[str] = Future()
    async with anyio.create_task_group() as task_group:
        task_group.start_soon(find_app_link_in_logs, link_fut)
        await anyio.sleep(0.5)
        task_group.start_soon(hdc.click_by_bounds, utils.parse_bounds(gallery_view_btn))

    await link_fut.wait()
    parse_res = utils.parse_input_split_links_pkgs_and_app_ids(link_fut.result())
    await anyio.sleep(1)
    await hdc.click_by_bounds(utils.parse_bounds(back_btn))
    if parse_res.empty():
        logger.error(f"没有找到包名 [{app_name}]")
        return
    
    pkg = parse_res.pkgs[-1] # get last pkg
    logger.success(f"应用 [{app_name}] 包名 [{pkg}]")
    


    return pkg

async def pull_apps_in_categories():
    full_apps_list: set[str] = set()
    stable_count = 0
    while stable_count < 2:
        await anyio.sleep(0.25)
        res = await hdc.dump_layout_to_json()
        list_items = utils.find_json_value_by_prev_path(res, utils.find_json_value_as_path(res, "List")[0], 2)
        
        current_apps_list: list[str] = []
        current_apps_bounds: dict[str, tuple[int, int, int, int]] = {}
        for path in utils.find_json_value_as_path(list_items, "app_name"):
            item = utils.find_json_value_by_prev_path(list_items, path, 1)
            try:
                text = item['text']
            except Exception:
                logger.traceback("无法获取应用名称，跳过该应用", item)
                continue
            if text is None or text == '' or text in current_apps_list or text in full_apps_list:
                continue
            current_apps_list.append(text)
            bounds = utils.parse_bounds(item['bounds'])
            current_apps_bounds[text] = bounds
    
        new_diff_apps = set(current_apps_list) - full_apps_list
        not_exists_apps = await get_not_exists_apps(list(new_diff_apps))
        new_shared = []
        for app in current_apps_list:
            if app not in not_exists_apps:
                continue
            logger.success(f"发现新应用 [{app}]")
            await anyio.sleep(0.25)
            shared_res = await click_app_in_category_and_share(app, current_apps_bounds[app])
            if not shared_res:
                logger.error(f"应用 [{app}] 分享不了？")
                continue
            new_shared.append(shared_res)

        if submit_in_python:
            await gallery.get_gallery().submit_apps(*new_shared, comment=gallery.CommentInfo(
                user=submit_username,
            ))

        for app in current_apps_list:
            full_apps_list.add(app)

        # roll down
        main_screen_size = await hdc.get_main_screen_size()
        await hdc.roll_to_y(main_screen_size[0] * 0.5, main_screen_size[1] * 0.2, main_screen_size[1] * 0.8)

        if not current_apps_list:
            stable_count += 1
            logger.debug(f"尝试滑动失败，可能是到达底部 [{stable_count}]")
            continue
        
        stable_count = 0

async def next_pull_apps_in_categories():
    apps_category_res = await hdc.dump_layout_to_json()
    # __NavdestinationField__Text__MainTitle__
    path: utils.JSON_PATH = utils.find_json_value_as_path(apps_category_res, "__NavdestinationField__Text__MainTitle__")[0][:-3] + [0, 'attributes']

    await pull_apps_in_categories()


    exit_btn = utils.parse_bounds(utils.find_json_value_by_path(apps_category_res, path)['bounds'])
    await hdc.click_by_bounds(exit_btn, 0.25)
        

async def main(args):
    global skip_categories, submit_username, submit_in_python, skip_check_apps
    logger.info("AppGallery Ciallo～ (∠・ω< )⌒★")
    logger.info("请确保当前在 [应用市场] 首页~")

    # skip categories
    args_skip_categories = args.skip_categories
    if args_skip_categories is not None and isinstance(args_skip_categories, list) and len(args_skip_categories) > 0:
        for sc in args_skip_categories:
            if sc in skip_categories:
                continue
            skip_categories.append(sc)

    args_gallery_api = args.gallery_api or "https://hmos.txit.top/api"
    logger.info(f"Gallery API: {args_gallery_api}")
    gallery.init_gallery(args_gallery_api)

    
    args_username = args.username
    if args_username is None or not args_username.strip():
        logger.info("未找到用户名")
        return
    
    submit_username = args_username
    logger.info(f"用户名 [{submit_username}]")

    args_submit = args.submit
    submit_in_python = args_submit
    if not args_submit:
        logger.info("将在 [应用看板] 提交数据")
    else:
        logger.info("将在 [Python] 提交数据")

    args_skip_check_apps = args.skip_check_apps
    if args_skip_check_apps:
        logger.info("跳过检查应用是否存在")
        skip_check_apps = True

    args_fast_pull = args.fast_pull
    if args_fast_pull:
        logger.info("快速模式，需要在应用分类中的应用列表")
        await next_pull_apps_in_categories()
        return

    await click_bottom_bar_app()
    await click_app_categories()
    await pull_app_in_categories()


    