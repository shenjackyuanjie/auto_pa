import argparse
from dataclasses import dataclass, field
import json
import os
from pathlib import Path
import random
import re
from typing import Any, Callable, Optional

import anyio

from core import common
from src import hdc, utils
from src.logger import logger


STATE_VERSION = 1
SEARCH_FIELD_KEY_PREFIX = "__SearchField__search_box"
SEARCH_BUTTON_KEY_PREFIX = "__SearchField__Button__search_box"
SEARCH_CANCEL_KEY_PREFIX = "__SearchField__CancelButton__search_box"
SEARCH_CONTAINER_KEY_PREFIX = "search_box"
SEARCH_RESULT_BACK_KEY = "SearchInputCard.Button.searchFrameBack"
FRESH_APPS_TEXT = "新鲜应用"
MAX_SCROLLS = 100
STABLE_SCROLLS = 1
SCROLL_BATCH_SIZE = 2
SCROLL_SCALE = 10
SCROLL_WAIT = 0.1
INPUT_SETTLE_WAIT = 0.3
FAST_RESULT_TIMEOUT = 6.0
MAX_SEARCH_ATTEMPTS = 3


def _attributes(node: Any) -> dict[str, Any]:
    if not isinstance(node, dict):
        return {}
    attrs = node.get("attributes")
    return attrs if isinstance(attrs, dict) else node


def _iter_nodes(node: Any):
    if isinstance(node, dict):
        yield node
        children = node.get("children")
        if isinstance(children, list):
            for child in children:
                yield from _iter_nodes(child)
    elif isinstance(node, list):
        for child in node:
            yield from _iter_nodes(child)


def find_bounds_by_key(layout: Any, key: str) -> Optional[str]:
    for node in _iter_nodes(layout):
        attrs = _attributes(node)
        bounds = attrs.get("bounds")
        if attrs.get("key") == key and isinstance(bounds, str):
            return bounds
    return None


def find_bounds_by_key_prefix(layout: Any, key_prefix: str) -> Optional[str]:
    for node in _iter_nodes(layout):
        attrs = _attributes(node)
        key = attrs.get("key")
        bounds = attrs.get("bounds")
        if (
            isinstance(key, str)
            and key.startswith(key_prefix)
            and isinstance(bounds, str)
        ):
            return bounds
    return None


def find_text_by_key_prefix(layout: Any, key_prefix: str) -> Optional[str]:
    for node in _iter_nodes(layout):
        attrs = _attributes(node)
        key = attrs.get("key")
        if not isinstance(key, str) or not key.startswith(key_prefix):
            continue
        text = attrs.get("text")
        return text if isinstance(text, str) else None
    return None


def find_bounds_by_text(layout: Any, text: str) -> Optional[str]:
    for node in _iter_nodes(layout):
        attrs = _attributes(node)
        bounds = attrs.get("bounds")
        if attrs.get("text") == text and isinstance(bounds, str):
            return bounds
    return None


def _list_app_snapshot(list_layout: Any) -> tuple[tuple[str, str], ...]:
    apps: list[tuple[str, str]] = []
    seen: set[tuple[str, str]] = set()
    for node in _iter_nodes(list_layout):
        attrs = _attributes(node)
        if attrs.get("key") != "app_name":
            continue
        text = attrs.get("text")
        bounds = attrs.get("bounds")
        if not isinstance(text, str) or not text or not isinstance(bounds, str):
            continue
        item = (text, bounds)
        if item not in seen:
            seen.add(item)
            apps.append(item)
    return tuple(apps)


def find_app_list(layout: Any) -> Any | None:
    candidates: list[tuple[int, int, Any]] = []
    for index, path in enumerate(utils.find_json_value_as_path(layout, "List")):
        list_layout = utils.find_json_value_by_prev_path(layout, path, 2)
        candidates.append((len(_list_app_snapshot(list_layout)), index, list_layout))
    if not candidates:
        return None
    score, _, app_list = max(candidates, key=lambda item: (item[0], -item[1]))
    return app_list if score > 0 else None


def app_snapshot(layout: Any) -> tuple[tuple[str, str], ...]:
    app_list = find_app_list(layout)
    return _list_app_snapshot(app_list) if app_list is not None else ()


def _deduplicate(values: list[str]) -> list[str]:
    return list(dict.fromkeys(value for value in values if value))


@dataclass
class SearchState:
    collection_complete: bool = False
    app_names: list[str] = field(default_factory=list)
    searched_names: list[str] = field(default_factory=list)

    def add_apps(self, names: list[str] | set[str]):
        self.app_names = _deduplicate([*self.app_names, *names])

    def mark_searched(self, name: str):
        self.searched_names = _deduplicate([*self.searched_names, name])

    def to_dict(self) -> dict[str, Any]:
        return {
            "version": STATE_VERSION,
            "collection_complete": self.collection_complete,
            "app_names": self.app_names,
            "searched_names": self.searched_names,
        }

    @classmethod
    def from_dict(cls, value: Any) -> "SearchState":
        if not isinstance(value, dict) or value.get("version") != STATE_VERSION:
            raise ValueError("搜索进度文件版本不兼容，请使用 --fresh 重新开始")
        collection_complete = value.get("collection_complete")
        app_names = value.get("app_names")
        searched_names = value.get("searched_names")
        if not isinstance(collection_complete, bool):
            raise ValueError("搜索进度文件 collection_complete 无效")
        if not isinstance(app_names, list) or not all(
            isinstance(item, str) for item in app_names
        ):
            raise ValueError("搜索进度文件 app_names 无效")
        if not isinstance(searched_names, list) or not all(
            isinstance(item, str) for item in searched_names
        ):
            raise ValueError("搜索进度文件 searched_names 无效")
        return cls(
            collection_complete=collection_complete,
            app_names=_deduplicate(app_names),
            searched_names=_deduplicate(searched_names),
        )


class SearchStateStore:
    def __init__(self, path: Path):
        self.path = path

    def load(self, fresh: bool = False) -> SearchState:
        if fresh or not self.path.exists():
            return SearchState()
        try:
            value = json.loads(self.path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            raise ValueError(
                f"无法读取搜索进度 [{self.path}]，请使用 --fresh 重新开始"
            ) from exc
        return SearchState.from_dict(value)

    def save(self, state: SearchState):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        temp_path = self.path.with_suffix(f"{self.path.suffix}.tmp")
        temp_path.write_text(
            json.dumps(state.to_dict(), ensure_ascii=False, indent=2),
            encoding="utf-8",
        )
        os.replace(temp_path, self.path)


def state_path_for_device(sn: str, random_mode: bool = False) -> Path:
    safe_sn = re.sub(r"[^A-Za-z0-9_.-]+", "_", sn).strip("._") or "device"
    suffix = ".random" if random_mode else ""
    return Path(".cache") / "search" / f"{safe_sn}{suffix}.json"


class AppGallerySearchDevice(common.AppGalleryCommonDevice):
    def __init__(
        self,
        device: hdc.Device,
        state: SearchState,
        state_store: SearchStateStore,
        random_mode: bool = False,
    ):
        super().__init__(device)
        self.state = state
        self.state_store = state_store
        self.random_mode = random_mode
        self._home_layout: Any = None

    async def share_app(self, app: str) -> bool:
        return False

    async def share_apps(
        self, apps: list[str], apps_pos: dict[str, str]
    ) -> list[str]:
        return []

    async def _wait_for_layout(
        self,
        predicate: Callable[[Any], bool],
        description: str,
        timeout: float = 12.0,
        interval: float = 0.35,
    ) -> Any:
        with anyio.fail_after(timeout):
            while True:
                layout = await self.dump_layout_to_json()
                if predicate(layout):
                    return layout
                await anyio.sleep(interval)
        raise RuntimeError(f"[{self.tag}] 等待 [{description}] 超时")

    async def _scroll_app_list_to_end(
        self, allow_empty: bool = False, initial_layout: Any = None
    ) -> tuple[list[str], Any]:
        names: list[str] = []
        seen_names: set[str] = set()
        previous: tuple[tuple[str, str], ...] | None = None
        stable = 0
        last_layout: Any = None

        for _ in range(MAX_SCROLLS):
            if initial_layout is not None:
                last_layout = initial_layout
                initial_layout = None
            else:
                last_layout = await self.dump_layout_to_json()
            snapshot = app_snapshot(last_layout)
            for name, _ in snapshot:
                if name not in seen_names:
                    seen_names.add(name)
                    names.append(name)

            if not snapshot and not allow_empty:
                await anyio.sleep(0.5)
                continue

            if snapshot == previous:
                stable += 1
                if stable >= STABLE_SCROLLS:
                    return names, last_layout
            else:
                stable = 0
            previous = snapshot
            for _ in range(SCROLL_BATCH_SIZE):
                await self.simple_roll_down(
                    0.5, 0.175, SCROLL_SCALE, wait_for=SCROLL_WAIT
                )

        raise RuntimeError(f"[{self.tag}] 滑动应用列表超过 [{MAX_SCROLLS}] 次仍未到底")

    async def _collect_current_app_list(self, exit_after: bool = True) -> list[str]:
        names, layout = await self._scroll_app_list_to_end()
        if exit_after:
            await self.go_back(layout, wait_for=1.5)
        return names

    async def custom_pull_apps(
        self, btn_pos: str, category: str
    ) -> common.PullResult:
        before = len(self.state.app_names)
        await self.click_by_bounds(btn_pos, 1.5)

        if self.random_mode:
            fresh_apps_bounds = None
            for _ in range(4):
                layout = await self.dump_layout_to_json()
                fresh_apps_bounds = find_bounds_by_text(layout, FRESH_APPS_TEXT)
                if fresh_apps_bounds is not None:
                    break
                await self.simple_roll_down(0.5, 0.2, 0.72)
            if fresh_apps_bounds is None:
                raise RuntimeError(
                    f"[{self.tag}] 分类 [{category}] 未找到 [{FRESH_APPS_TEXT}]"
                )
            logger.info(
                f"[{self.tag}] 正在收集分类 [{category}] 的 [{FRESH_APPS_TEXT}]"
            )
            await self.click_by_bounds(fresh_apps_bounds, 1.5)

        category_apps = await self._collect_current_app_list()

        if self.random_mode:
            await self.go_back(wait_for=1.5)

        self.state.add_apps(category_apps)
        self.state_store.save(self.state)
        added = len(self.state.app_names) - before
        logger.info(
            f"[{self.tag}] 分类 [{category}] 收集 [{len(category_apps)}] 个名称，新增 [{added}] 个"
        )
        return common.PullResult(total=len(category_apps), new=added)

    async def collect_app_names(self):
        if self.state.collection_complete:
            logger.info(
                f"[{self.tag}] 已从进度恢复 [{len(self.state.app_names)}] 个应用名称"
            )
            return

        await self.start_app()
        pages = [("应用", self.go_app_page)]
        if not self.random_mode:
            pages.append(("游戏", self.go_game_page))
        for page_name, go_page in pages:
            logger.info(f"[{self.tag}] 开始收集 [{page_name}] 分类")
            await go_page()
            await self.go_categories_page()
            await self.pull_categories()

        self.state.collection_complete = True
        self.state_store.save(self.state)
        logger.success(
            f"[{self.tag}] 名称收集完成，共 [{len(self.state.app_names)}] 个唯一名称"
        )

    async def _ensure_app_home(self):
        await self.start_app()
        await self.go_app_page()
        self._home_layout = await self._wait_for_layout(
            lambda layout: find_bounds_by_key_prefix(
                layout, SEARCH_FIELD_KEY_PREFIX
            )
            is not None,
            "应用主页搜索框",
        )

    async def _wait_for_search_result(self, app_name: str) -> Any:
        is_result_page = (
            lambda layout: find_bounds_by_key(layout, SEARCH_RESULT_BACK_KEY)
            is not None
        )
        try:
            return await self._wait_for_layout(
                is_result_page,
                f"[{app_name}] 搜索结果页",
                timeout=FAST_RESULT_TIMEOUT,
            )
        except TimeoutError:
            logger.warning(
                f"[{self.tag}] [{app_name}] 快速搜索未跳转，使用当前 UI 重试"
            )

        layout = await self.dump_layout_to_json()
        if is_result_page(layout):
            return layout

        input_bounds = find_bounds_by_key_prefix(layout, SEARCH_FIELD_KEY_PREFIX)
        search_button = find_bounds_by_key_prefix(layout, SEARCH_BUTTON_KEY_PREFIX)
        if input_bounds is None or search_button is None:
            raise RuntimeError(f"[{self.tag}] [{app_name}] 搜索浮层已丢失")

        current_text = find_text_by_key_prefix(layout, SEARCH_FIELD_KEY_PREFIX)
        if current_text != app_name:
            cancel_bounds = find_bounds_by_key_prefix(
                layout, SEARCH_CANCEL_KEY_PREFIX
            )
            if current_text and cancel_bounds is not None:
                await self.click_by_bounds(cancel_bounds, 0.1)
            await self.device.input_text_by_bounds(input_bounds, app_name)
            await anyio.sleep(INPUT_SETTLE_WAIT)

        await self.click_by_bounds(search_button, 0.1)
        return await self._wait_for_layout(
            is_result_page,
            f"[{app_name}] 重试搜索结果页",
            timeout=15.0,
        )

    async def _search_once(self, app_name: str) -> int:
        home_layout = self._home_layout
        if home_layout is None:
            home_layout = await self._wait_for_layout(
                lambda layout: find_bounds_by_key_prefix(
                    layout, SEARCH_FIELD_KEY_PREFIX
                )
                is not None,
                "应用主页搜索框",
            )
        self._home_layout = None
        search_field = find_bounds_by_key_prefix(home_layout, SEARCH_FIELD_KEY_PREFIX)
        assert search_field is not None
        search_container = find_bounds_by_key_prefix(
            home_layout, SEARCH_CONTAINER_KEY_PREFIX
        )
        if search_container is None:
            raise RuntimeError(f"[{self.tag}] 未找到搜索容器")
        _, y1, x2, y2 = utils.parse_bounds(search_container)
        search_button = (x2 - 120, y1, x2, y2)

        await self.click_by_bounds(search_field, 0.2)
        await self.device.input_text_by_bounds(search_field, app_name)
        await anyio.sleep(INPUT_SETTLE_WAIT)
        await self.click_by_bounds(search_button, 0.1)
        result_layout = await self._wait_for_search_result(app_name)

        try:
            if not app_snapshot(result_layout):
                result_layout = await self._wait_for_layout(
                    lambda layout: bool(app_snapshot(layout)),
                    f"[{app_name}] 搜索结果",
                    timeout=6.0,
                )
            result_names, result_layout = await self._scroll_app_list_to_end(
                allow_empty=True, initial_layout=result_layout
            )
        except TimeoutError:
            result_names = []
            result_layout = await self.dump_layout_to_json()
            logger.warning(f"[{self.tag}] [{app_name}] 没有搜索结果")

        back_bounds = find_bounds_by_key(result_layout, SEARCH_RESULT_BACK_KEY)
        if back_bounds is None:
            raise RuntimeError(f"[{self.tag}] 未找到搜索结果页返回按钮")
        await self.click_by_bounds(back_bounds, 0.1)
        self._home_layout = await self._wait_for_layout(
            lambda layout: find_bounds_by_key_prefix(
                layout, SEARCH_FIELD_KEY_PREFIX
            )
            is not None,
            "返回应用主页",
        )
        return len(result_names)

    async def search_all(self) -> list[str]:
        searched = set(self.state.searched_names)
        pending = [name for name in self.state.app_names if name not in searched]
        if self.random_mode:
            random.shuffle(pending)
        if not pending:
            logger.success(f"[{self.tag}] 所有应用名称均已搜索完成")
            return []

        await self._ensure_app_home()
        failed: list[str] = []
        total = len(self.state.app_names)
        for app_name in pending:
            completed = False
            for attempt in range(1, MAX_SEARCH_ATTEMPTS + 1):
                try:
                    logger.info(
                        f"[{self.tag}] 正在搜索 [{app_name}] "
                        f"[{len(self.state.searched_names) + 1}/{total}]"
                    )
                    result_count = await self._search_once(app_name)
                    self.state.mark_searched(app_name)
                    self.state_store.save(self.state)
                    logger.success(
                        f"[{self.tag}] [{app_name}] 搜索完成，结果页发现 "
                        f"[{result_count}] 个名称"
                    )
                    completed = True
                    break
                except Exception:
                    logger.traceback(
                        f"[{self.tag}] [{app_name}] 第 [{attempt}] 次搜索失败"
                    )
                    if attempt < MAX_SEARCH_ATTEMPTS:
                        await self._ensure_app_home()
            if not completed:
                failed.append(app_name)
        return failed


async def run_device(device: hdc.Device, args: argparse.Namespace):
    random_mode = getattr(args, "random", False)
    store = SearchStateStore(state_path_for_device(device.sn, random_mode))
    state = store.load(fresh=args.fresh)
    if args.fresh:
        store.save(state)

    search_device = AppGallerySearchDevice(
        device, state, store, random_mode=random_mode
    )
    common.global_var[device.sn].phone = device.device_type == "phone"
    failed: list[str] = []
    try:
        await search_device.collect_app_names()
        failed = await search_device.search_all()
    finally:
        await device.close_app(common.APPGALLERY_PKG)

    if failed:
        display = ", ".join(f"[{name}]" for name in failed)
        raise RuntimeError(
            f"[{device.tag}] [{len(failed)}] 个名称搜索失败，下次运行会重试：{display}"
        )


async def main(args: argparse.Namespace):
    devices = await hdc.get_devices()
    if not devices:
        raise RuntimeError("未找到已连接设备")
    for device in devices:
        logger.info(f"设备信息 [{device.tag}]")

    async with anyio.create_task_group() as task_group:
        for device in devices:
            task_group.start_soon(run_device, device, args)
