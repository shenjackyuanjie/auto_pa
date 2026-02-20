import anyio
from src.logger import logger
from src import hmgallery as gallery, hdc, utils
from core import common
from tianxiu2b2t.utils import runtime
from tianxiu2b2t.units import format_count_time
from graceful_shutdown import ShutdownProtection

username: str = ""

class AppGalleryHilogDevice(common.AppGalleryCommonDevice):
    def __init__(self, device: hdc.Device):
        super().__init__(device)

    async def custom_pull_apps(self, btn_pos: str, category: str) -> common.PullResult:
        exit_btn = None
        async def pull_chunk_in_categories():
            nonlocal exit_btn, layout
            if exit_btn is None:
                exit_btn = utils.find_json_value_by_prev_path(
                    layout, utils.find_json_value_as_path(layout, "BackButton")[0]
                )["bounds"]
            clicked_chunks: set[str] = set()
            retries = 0
            while len(clicked_chunks) < len(common.FUCKOFF_SUB_CHUNKS):
                current_chunks = len(clicked_chunks)
                layout = await self.dump_layout_to_json()
                if exit_btn is None:
                    exit_btn = utils.find_json_value_by_prev_path(
                        layout, utils.find_json_value_as_path(layout, "BackButton")[0]
                    )["bounds"]
                for chunk in common.FUCKOFF_SUB_CHUNKS:
                    # if chunk in clicked_chunks:
                    #     continue
                    chunk_paths = utils.find_json_value_as_path(layout, chunk)
                    if len(chunk_paths) == 0:
                        continue
                    chunk_path = chunk_paths[0]
                    match_chunk = utils.find_json_value_by_prev_path(layout, chunk_path)["text"]
                    logger.info(f"[{self.tag}] 正在拉取分类 [{category}] 的 [{match_chunk}]...")
                    await self.click_by_bounds(
                        utils.find_json_value_by_prev_path(layout, chunk_path)["bounds"],
                        1.75,
                    )
                    await anyio.sleep(1 + common.ping * 0.05)
                    await inner_app_pulls()
                    clicked_chunks.add(match_chunk)
                    # roll
                if current_chunks == len(clicked_chunks):
                    if retries >= 3:
                        display_chunks = ", ".join(map(lambda x: f"[{x}]", clicked_chunks))
                        logger.warning(f"[{self.tag}] [{category}] 怎么只有 {display_chunks} 呢？")
                        break
                    retries += 1
                await self.simple_roll_down(0.5, 0.2, 0.72)

        
        
        async def inner_app_pulls():
            apps = []
            exit_btn = None
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
            exit_btn = utils.find_json_value_by_prev_path(
                layout, utils.find_json_value_as_path(layout, "BackButton")[0]
            )["bounds"]
            await self.click_by_bounds(exit_btn)

        pulled_apps: set[str] = set()
        new_apps: set[str] = set()
        async def poll_appgallery():
            with ShutdownProtection():
                async for line in appgallery_hilog:
                    res = hdc.parse_hilog_line(line)
                    if res is None:
                        continue
                    log = res.log
                    try:
                        pkg = log.split("GetSpecifiedDistributionType failed -n ", 1)[1].split(" ret:")[0]
                    except IndexError:
                        logger.warning(f"解析日志失败: [{log}]")
                        continue
                    if pkg in pulled_apps:
                        continue
                    pulled_apps.add(pkg)
                    logger.debug(f"AppGallery: {pkg}")
                    tg.start_soon(submit_app, pkg)

        async def submit_app(
            data: str
        ):
            res = await gallery.get_gallery().exists_app(data) if not common.skip_app_check else None 
            if res is not None:
                logger.debug(f"已存在 [{res.name} ({res.app_id} - {res.pkg_name})]")
                return
            logger.debug(f"正在提交 [{data}]")
            res = await gallery.get_gallery().submit_app(data, None, gallery.CommentInfo(
                user=username
            ))
            if res is None:
                logger.error(f"提交失败 [{data}]")
                return
            logger.success(f"提交成功 [{data}]")
            new_apps.add(data)
            
            
        start_time = runtime.perf_counter_ns()
        async with hdc.HilogProcess(
            self.device_id,
            "-e", "GetSpecifiedDistributionType"
        ) as appgallery_hilog, anyio.create_task_group() as tg:
            tg.start_soon(poll_appgallery)
            
            await self.click_by_bounds(btn_pos)
            await anyio.sleep(0.15)

            layout = await self.dump_layout_to_json()
            pre_exit_btn = utils.find_json_value_by_prev_path(
                layout, utils.find_json_value_as_path(layout, "BackButton")[0]
            )["bounds"]
            new_ui = await self.get_new_ui()
            if not new_ui:
                await inner_app_pulls()
            else:
                await pull_chunk_in_categories()

            await appgallery_hilog.exit()

        end_time = runtime.perf_counter_ns()
        elapsed_time = end_time - start_time
        avg_apps = elapsed_time / len(pulled_apps) if len(pulled_apps) > 0 else 0
        avg_new_apps = elapsed_time / len(new_apps) if len(new_apps) > 0 else 0
        display_category = f"[{category}] " if category else ""
        logger.info(
            f"[{self.tag}] {display_category}拉取应用完成, 共 [{len(pulled_apps)}] 个应用，新应用 [{len(new_apps)}] 个，耗时 [{format_count_time(elapsed_time)}] 平均 [{format_count_time(avg_apps)}/个] 新应用平均 [{format_count_time(avg_new_apps)}/个]"
        )
        await self.click_by_bounds(pre_exit_btn)

        return common.PullResult(
            total=len(pulled_apps),
            new=len(new_apps)
        )

    def share_app(self, app: str) -> bool:
        return False
    
    def share_apps(self, apps: list[str], apps_pos: dict[str, str]) -> list[str]:
        return []

@common.device_main(device_class=AppGalleryHilogDevice)
async def device_main(
    device: common.AppGalleryCommonDevice
):
    await device.start_app()
    for go_page in [device.go_app_page, device.go_game_page]:
        await go_page()
        await device.go_categories_page()
        await device.pull_categories()




@common.loop_main
async def inner_main():
    await device_main

async def main(args):
    global username
    logger.info("AppGallery Pull Ciallo～ (∠・ω< )⌒★")
    await common.init(args)

    username = args.username
    
    logger.info(f"App Gallery API [{common.gallery_base_url}]")
    if common.skip_app_check:
        logger.info("跳过应用检查")

    if (username is None or not username.strip()):
        logger.error("无效 username")
        return

    await inner_main()

    logger.info("AppGallery Pull 结束")
