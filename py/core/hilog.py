import anyio
from py.src.logger import logger
from py.src import hmgallery as gallery, hdc, utils
from py.core import common
from tianxiu2b2t.utils import runtime
from tianxiu2b2t.units import format_count_time
from graceful_shutdown import ShutdownProtection

username: str = ""
submit_interval: float = 1.0

class AppGalleryHilogDevice(common.AppGalleryCommonDevice):
    def __init__(self, device: hdc.Device):
        super().__init__(device)

    async def custom_pull_apps(self, btn_pos: str, category: str) -> common.PullResult:
        layout = None

        async def pull_chunk_in_categories():
            nonlocal layout
            clicked_chunks: set[str] = set()
            retries = 0
            while len(clicked_chunks) < len(common.FUCKOFF_SUB_CHUNKS):
                current_chunks = len(clicked_chunks)
                layout = await self.dump_layout_to_json()
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

        async def inner_app_pulls(exit_after_pull: bool = True):
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
                    ui_apps.add(text)
                    cur_apps.append(text)
                    apps_pos[text] = app_pos
                    logger.debug(f"[{self.tag}] [{text}] [{app_pos}]")

                for _ in range(2):
                    await self.device.simple_roll_down(0.5, 0.175, 10)
                if current_apps_len == len(apps):
                    break
                # break
            if exit_after_pull:
                await self.go_back(layout)

        pulled_apps: set[str] = set()
        ui_apps: set[str] = set()
        new_apps: set[str] = set()
        submit_send, submit_recv = anyio.create_memory_object_stream(512)

        async def poll_appgallery():
            with ShutdownProtection():
                async with submit_send:
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
                        await submit_send.send(pkg)

        async def submit_pending_apps():
            last_submit_started_at: float | None = None
            async with submit_recv:
                async for data in submit_recv:
                    if submit_interval > 0 and last_submit_started_at is not None:
                        wait_for = last_submit_started_at + submit_interval - runtime.perf_counter()
                        if wait_for > 0:
                            await anyio.sleep(wait_for)
                    last_submit_started_at = runtime.perf_counter()
                    await submit_app(data)

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

        async def run_ui_operations():
            await self.click_by_bounds(btn_pos)
            await anyio.sleep(0.15)

            layout = await self.dump_layout_to_json()
            new_ui = await self.get_new_ui()
            if not new_ui:
                await inner_app_pulls(exit_after_pull=False)
            else:
                await pull_chunk_in_categories()

        if common.no_submit:
            logger.info(f"[{self.tag}] No Submit 模式：跳过 hilog 抓取和应用提交")
            await run_ui_operations()
        else:
            async with hdc.HilogProcess(
                self.device_id,
                "-e", "GetSpecifiedDistributionType"
            ) as appgallery_hilog, anyio.create_task_group() as tg:
                tg.start_soon(poll_appgallery)
                tg.start_soon(submit_pending_apps)

                await run_ui_operations()

                await appgallery_hilog.exit()

        end_time = runtime.perf_counter_ns()
        elapsed_time = end_time - start_time
        total_apps = len(ui_apps) if common.no_submit else len(pulled_apps)
        avg_apps = elapsed_time / total_apps if total_apps > 0 else 0
        avg_new_apps = elapsed_time / len(new_apps) if len(new_apps) > 0 else 0
        display_category = f"[{category}] " if category else ""
        logger.info(
            f"[{self.tag}] {display_category}拉取应用完成, 共 [{total_apps}] 个应用，新应用 [{len(new_apps)}] 个，耗时 [{format_count_time(elapsed_time)}] 平均 [{format_count_time(avg_apps)}/个] 新应用平均 [{format_count_time(avg_new_apps)}/个]"
        )
        await self.go_back(wait_for=1.75)

        return common.PullResult(
            total=total_apps,
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
    global username, submit_interval
    logger.info("AppGallery Pull Ciallo～ (∠・ω< )⌒★")
    await common.init(args)

    username = args.username
    raw_submit_interval = getattr(args, "submit_interval", 1.0)
    if raw_submit_interval < 0:
        logger.warning(f"提交间隔不能小于 0，已改为 [0.00s]")
    submit_interval = max(raw_submit_interval, 0.0)
    
    logger.info(f"App Gallery API [{common.gallery_base_url}]")
    if common.no_submit:
        logger.info("No Submit 模式：仅执行 UI 操作，不抓取 hilog，不提交应用")
    if common.skip_app_check:
        logger.info("跳过应用检查")
    if common.no_submit:
        pass
    elif submit_interval > 0:
        logger.info(f"Hilog 提交最小间隔 [{submit_interval:.2f}s]")
    else:
        logger.info("Hilog 提交限速已关闭")

    if not common.no_submit and (username is None or not username.strip()):
        logger.error("无效 username")
        return

    await inner_main()

    logger.info("AppGallery Pull 结束")
