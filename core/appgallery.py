import argparse
from dataclasses import dataclass
import random
from typing import Optional
import anyio
import src.hdc as hdc
import src.utils as utils
import src.hmgallery as gallery
from src.logger import logger


argument = argparse.ArgumentParser()
argument.add_argument('--skip-categories', '--sc', type=str, nargs="+")
argument.add_argument('--fast-pull', '--fp', help="Fast pull app, not check devices in main page", action="store_true")
argument.add_argument('--gallery-api', '--ga', help="Use gallery api to pull app", default="https://hmos.txit.top/api")
argument.add_argument('--username', '-u', help="Submit app with this username", required=True)

@dataclass
class AppInCategory:
    name: str
    bounds: tuple[float, float, float, float]

main_layout_res = None
app_categories_res = None
app_categories_size = None
app_categories_branches: list[utils.JSON_PATH] = [
    ['children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 2, 'children', 0, 'children', 0, 'children', 0, 'children', 1, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children'],
    ['children', 0, 'children'],
]
apps_in_category_path: utils.JSON_PATH = ['children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 1, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children',]
apps_in_category_app_name = ['children', 0, 'attributes', 'text']

skip_categories = []
submit_username = ''

async def is_main_page():
    main_screen = await hdc.get_main_screen_size()
    res = await hdc.dump_layout_to_json()
    paths = utils.find_json_value_as_path(res, "探索")
    try:
        value = utils.find_json_value_by_path(res, paths[2][:-1])
    except Exception as e:
        logger.error(f"Error: {e}")
        with open("layout.json", "w", encoding="utf-8") as f:
            f.write(utils.json_dumps(res))
        return False
    bounds = utils.parse_bounds(value['bounds'])
    phone_mode = await hdc.is_phone_mode()
    if phone_mode:     # 判断是不是在屏幕下方
        result = bounds[1] > main_screen[1] * 0.8
    else:
        result = bounds[1] < main_screen[0] * 0.2
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
    # if await is_app_categories_page():
    #     return
    paths = utils.find_json_value_as_path(app_categories_res, "分类")
    value = utils.find_json_value_by_path(app_categories_res, paths[0][:-1])
    bounds = utils.parse_bounds(value['bounds'])
    await hdc.click_pos((bounds[0] + bounds[2]) / 2, (bounds[1] + bounds[3]) / 2)

async def get_app_categories():
    await wait_for_app_categories_page()
    global app_categories_res
    if app_categories_res is None:
        await wait_for_app_categories_page()

    # with open("output.txt", "w", encoding="utf-8") as f:
    #     daohang_paths = utils.find_json_value_as_path(app_categories_res, "出行导航")
    #     children_paths = utils.find_json_value_as_path(app_categories_res, "儿童")
    #     home_paths = utils.find_json_value_as_path(app_categories_res, "房产与装修")
    #     diff_branch = []
    #     for idx, (dps, cps, hps) in enumerate(zip(daohang_paths, children_paths, home_paths)):
    #         for i, (dp, cp, hp) in enumerate(zip(dps, cps, hps)):
    #             if dp == cp == hp:
    #                 continue
    #             diff_branch.append(i)

    #             f.write(f"开始不同分支！{i}:\n")
    #             f.write(f"daohang: {dp}\n")
    #             f.write(f"children: {cp}\n")
    #             f.write(f"home: {hp}\n")
    #             f.write(f"common: {dps[:i]}\n")
                

    #         # 所以总结一下
    #         f.write("总结：\n")
    #         last_i = 0
    #         for i in diff_branch:
    #             f.write(f"分支{i}:\n")
    #             f.write(f"{dps[last_i:i]}\n")
    #             last_i = i

    #         f.write("最后一组:\n")
    #         f.write(f"{dps[last_i:]}")

    #         diff_branch.clear()

    # stable_count = 0
    # results = set()
    # while (stable_count := stable_count + 1) <= 3:
    #     res = await hdc.dump_layout_to_json()
    #     # 最后一个必然是


    # result = []  # 最终收集到的所有唯一分类文本
    # stable_count = 0  # 用于计数连续相同的次数
    # required_stable_times = 2  # 要求连续相同的次数，达到则退出循环

    # while stable_count < required_stable_times:
    #     # 1. 每次循环都获取最新布局并解析
    #     res = await hdc.dump_layout_to_json()
    #     values = utils.find_json_value_by_path(res, app_categories_start_path)

    #     # 2. 初始化本次循环的临时收集列表
    #     current_loop_items = []
    #     for value in values:
    #         val = utils.find_json_value_by_path(value, app_categories_middle_path)
    #         for v in val:
    #             print
    #             current_loop_items.append(utils.find_json_value_by_path(v, app_categories_end_path))

    #     # 3. 判断本次收集的结果是否与“最终结果”相同
    #     # 将列表排序后比较，避免顺序不同导致的误判
    #     if sorted(current_loop_items) == sorted(result):
    #         # 如果相同，稳定计数器加1
    #         stable_count += 1
    #         print(f"第 {stable_count} 次获取到相同结果。")
    #     else:
    #         # 如果不同，更新最终结果，并重置稳定计数器
    #         result = current_loop_items[:]  # 使用切片创建副本
    #         stable_count = 0
    #         print(f"结果更新，当前分类数: {len(result)}")

    #     main_screen_size = await hdc.get_main_screen_size()
    #     await hdc.roll_pos_y(main_screen_size[0] * 0.5, main_screen_size[1] * 0.5, 60)

    #     # 可选：增加短暂延迟，避免过于频繁的请求
    #     await anyio.sleep(0.5)

    # # 循环结束，输出最终稳定的结果
    # print(f"页面稳定，最终收集到的分类: {result}")

async def pull_app_in_categories():
    await wait_for_app_categories_page()
    global app_categories_res
    if app_categories_res is None:
        await wait_for_app_categories_page()


    # with open("output.txt", "w", encoding="utf-8") as f:
    #     daohang_paths = utils.find_json_value_as_path(app_categories_res, "出行导航")
    #     children_paths = utils.find_json_value_as_path(app_categories_res, "儿童")
    #     home_paths = utils.find_json_value_as_path(app_categories_res, "房产与装修")
    #     with open("layouts.json", "w", encoding="utf-8") as f:
    #         f.write(utils.json_dumps({
    #             "daohang": daohang_paths,
    #             "kids": children_paths,
    #             "home": home_paths
    #         }))
    #     diff_branch = []
    #     for idx, (dps, cps, hps) in enumerate(zip(daohang_paths, children_paths, home_paths)):
    #         for i, (dp, cp, hp) in enumerate(zip(dps, cps, hps)):
    #             if dp == cp == hp:
    #                 continue
    #             diff_branch.append(i)

    #             f.write(f"开始不同分支！{i}:\n")
    #             f.write(f"daohang: {dp}\n")
    #             f.write(f"children: {cp}\n")
    #             f.write(f"home: {hp}\n")
    #             f.write(f"common: {dps[:i]}\n")
                

    #         # 所以总结一下
    #         f.write("总结：\n")
    #         last_i = 0
    #         for i in diff_branch:
    #             f.write(f"分支{i}:\n")
    #             f.write(f"{dps[last_i:i]}\n")
    #             last_i = i

    #         f.write("最后一组:\n")
    #         f.write(f"{dps[last_i:]}")

    #         diff_branch.clear()

    main_screen_size = await hdc.get_main_screen_size()
    
    clicked_categories: set[str] = set()
    stable_count = 0
    while (stable_count := stable_count + 1) <= 3:
        current_clicked_categories = set()
        res = await hdc.dump_layout_to_json()
        values = get_value_from_categories_res(res)
        if not values:
            continue
        for value in values:
            try:
                item = utils.find_json_value_by_path(value, ['children', 0, 'children', 0, 'children', 0, 'attributes'])
            except Exception:
                continue
            text = item['text']
            if text is None or text == '' or text in clicked_categories:
                continue
            bounds = utils.parse_bounds(item['bounds'])
            if bounds[3] > main_screen_size[1] * 0.9:
                break
            clicked_categories.add(text)
            current_clicked_categories.add(text)
            if text in skip_categories:
                logger.warning(f"跳过分类 [{text}]")
                continue
            logger.info(f"点击分类 [{text}]")
            await hdc.click_by_bounds(bounds)

            await anyio.sleep(0.75)
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
    # return apps
    result = await gallery.get_gallery().search_app_names_exists(*apps)
    not_exists_apps = []
    for app, exists in result.items():
        if exists:
            continue
        not_exists_apps.append(app)
    return not_exists_apps

async def click_app_in_category_and_share(
    app_name: str,
    bounds: tuple[int, int, int, int]
) -> Optional[str]:
    await hdc.click_by_bounds(bounds)

    layout = await hdc.dump_layout_to_json()
    back_btn = utils.find_json_value_by_prev_path(layout, utils.find_json_value_as_path(layout, "__NavdestinationField__BackButton__Back__")[0])['bounds']
    share_btn = utils.find_json_value_by_prev_path(layout, utils.find_json_value_as_path(layout, "detail_share_menu")[0])['bounds']
    if back_btn is None or share_btn is None:
        logger.error(f"[{app_name}] 没有找到返回按钮或分享按钮")
        return

    await anyio.sleep(0.75) # wait for network pull


    # =============== Temp ===============
    await hdc.click_by_bounds(utils.parse_bounds(share_btn))
    await anyio.sleep(0.5)

    share_layout = await hdc.dump_layout_to_json()
    # with open("share_layout.json", "w", encoding="utf-8") as f:
    #     f.write(utils.json_dumps(share_layout))

    share_to_gallery_view = utils.find_json_value_by_prev_path(share_layout, utils.find_json_value_as_path(share_layout, "应用看板")[0])['bounds']
    await hdc.click_by_bounds(utils.parse_bounds(share_to_gallery_view))

    share_with_gallery_view_page = await hdc.dump_layout_to_json()
    gallery_view_btn = utils.find_json_value_by_prev_path(share_with_gallery_view_page, utils.find_json_value_as_path(share_with_gallery_view_page, "按已有信息投稿到看板")[0])['bounds']
    await hdc.click_by_bounds(utils.parse_bounds(gallery_view_btn))
    
    hilog = await hdc.hilog("-e", '"share"', "-T", "JSAPP", "-z", "10000")
    _, pkgs = utils.parse_input_split_links_and_pkgs_tuple(hilog)
    pkg = pkgs[-1] # get last pkg
    logger.success(f"应用 [{app_name}] 包名 [{pkg}]")
    # copy_btn = utils.find_json_value_by_prev_path(share_layout, utils.find_json_value_as_path(share_layout, "复制")[0])['bounds']
    # # quit_share_btn = utils.find_json_value_by_prev_path(share_layout, utils.find_json_value_as_path(share_layout, "arrow_button")[0])['bounds']
    # await hdc.click_by_bounds(utils.parse_bounds(copy_btn))
    # clipboards += 1
    # if clipboards % 10 == 0:
    #     logger.warning(f"剪贴板已使用 {clipboards} 次")
    # if clipboards >= clipboards_limit:
    #     # wait for console user input
    #     try:
    #         input("请手动清空剪贴板，然后按回车继续")
    #     except Exception:
    #         pass
    #     clipboards = 0 
    
    await anyio.sleep(0.3)
    # await hdc.click_by_bounds(utils.parse_bounds(back_btn))



    # =============== Temp ===============

    # SHAREEEEEEEEEEEEEE
    # and KNOCK BACK PREV PAGE
    # 你会很好奇为什么我不写这个分享逻辑
    # 因为某人在 2025/12/5 23:09:30 在某地铁上回家
    # 然后我现在需要等他回到家，写完这个玩意出来*
    # 我要哈气了

    await hdc.click_by_bounds(utils.parse_bounds(back_btn))

    return pkg

async def pull_apps_in_categories():
    full_apps_list: set[str] = set()
    stable_count = 0
    while stable_count <= 3:
        await anyio.sleep(0.25 + random.randint(3, 5) * 0.1)
        res = await hdc.dump_layout_to_json()
        data = utils.find_json_value_by_path(res, ['children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 1, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children', 0, 'children',])
        current_apps_list: list[str] = []
        current_apps_bounds: dict[str, tuple[int, int, int, int]] = {}
        for path in utils.find_json_value_as_path(data, "app_name"):
            item = utils.find_json_value_by_prev_path(data, path, 1)
            try:
                text = item['text']
            except Exception:
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
            shared_res = await click_app_in_category_and_share(app, current_apps_bounds[app])
            if not shared_res:
                logger.error(f"应用 [{app}] 分享不了？")
                continue
            new_shared.append(shared_res)
        
        await gallery.get_gallery().submit_apps(*new_shared, comment=gallery.CommentInfo(
            user=submit_username,
        ))

        for app in current_apps_list:
            full_apps_list.add(app)

        # roll down
        main_screen_size = await hdc.get_main_screen_size()
        await hdc.roll_to_y(main_screen_size[0] * 0.5, main_screen_size[1] * 0.2, main_screen_size[1] * 0.7375)

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

def get_value_from_categories_res(res: dict, idx: int = 0):
    paths = app_categories_branches[idx]
    value = utils.find_json_value_by_path(res, paths)
    if value is None:
        return []
    result = []
    next_idx = idx + 1
    for v in value:
        if not isinstance(v, dict) or next_idx >= len(app_categories_branches):
            result.append(v)
            continue
        result.extend(get_value_from_categories_res(v, next_idx))
    return result

        

async def main(args):
    global skip_categories, submit_username
    logger.info("AppGallery Ciallo～ (∠・ω< )⌒★")
    logger.info("请确保当前在应用市场首页~")

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

    args_fast_pull = args.fast_pull
    if args_fast_pull:
        logger.info("快速模式，需要在应用分类中的应用列表")
        await next_pull_apps_in_categories()
        return
    
    args_username = args.username
    if args_username is None or not args_username.strip():
        logger.info("未找到用户名")
        return
    
    submit_username = args_username
    logger.info(f"用户名: {submit_username}")

    await click_bottom_bar_app()
    await click_app_categories()
    await pull_app_in_categories()


    