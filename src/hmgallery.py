from dataclasses import asdict, dataclass
from typing import Any, Optional, TypedDict
import anyio
import httpx
from tianxiu2b2t.anyio.concurrency import gather

from .cache import cache
from .logger import logger

_gallery: Optional["HMGallery"] = None


class BaseResponse(TypedDict):
    success: bool
    data: Any
    total: Optional[int]
    limit: Optional[int]
    timestamp: str


class AppInfo(TypedDict):
    name: str


class SearchListResponse(TypedDict):
    data: list[AppInfo]
    page: int
    page_size: int
    total_count: int
    total_pages: int


@dataclass
class CommentInfo:
    platform: Optional[str] = None
    user: Optional[str] = None
    note: Optional[str] = None

    def clone(self):
        return CommentInfo(**asdict(self))

    def to_json(self):
        data = asdict(self)
        # pop value is None
        return {k: v for k, v in data.items() if v is not None}


@dataclass
class SearchParams:
    sort: str = "download_count"
    desc: bool = True
    page_size: int = 50
    search_key: str = "name"
    search_value: str = ""
    search_exact: bool = True


class HMGallery:
    def __init__(self, base_url: str):
        self.client = httpx.AsyncClient(base_url=base_url, http2=True)

    async def search_app_names_exists(
        self,
        *names: str,
    ) -> dict[str, bool]:
        return dict(
            zip(
                names,
                await gather(*(self.search_app_name_exists(name) for name in names)),
            )
        )

    async def _search_list(
        self,
        idx: int,
        name: str,
        params: SearchParams,
        _retries: int = 0,
    ) -> SearchListResponse:
        if _retries > 3:
            raise Exception("search list failed")
        try:
            resp = await self.client.get(f"apps/list/{idx}", params=asdict(params))
        except Exception as e:
            logger.warning(f"app [{name}] search list failed in retry {_retries}: {e}")
            return await self._search_list(idx, name, params, _retries + 1)
        if resp.status_code == 200:
            response: BaseResponse = resp.json()
            data: SearchListResponse = response["data"]
            return data
        return await self._search_list(idx, name, params, _retries + 1)

    async def search_app_name_exists(
        self,
        name: str,
    ) -> bool:
        if cache.get(f"app:{name}", False) is True:
            logger.debug(f"[{name}] is exists")
            return True
        params = {
            "sort": "download_count",
            "desc": True,
            "page_size": 50,
            "search_key": "name",
            "search_value": name,
            "search_exact": True,
        }
        idx = 0
        while 1:
            current_idx = (idx := idx + 1)
            try:
                data = await self._search_list(
                    current_idx, name, SearchParams(**params)
                )
            except Exception:
                logger.traceback(f"Failed to search app: {name}")
                continue
            # find name
            for app in data["data"]:
                if app["name"].lower() == name.lower():
                    cache.set(f"app:{name}", True, 3600)
                    return True
            if data["total_pages"] <= current_idx:
                return False
        return False

    async def submit_apps(
        self, *pkgs: str, comment: Optional[CommentInfo] = None
    ) -> dict[str, bool]:
        return dict(
            zip(
                pkgs,
                await gather(
                    *(
                        self.submit_app(pkg, comment.clone() if comment else None)
                        for pkg in pkgs
                    )
                ),
            )
        )

    async def _submit_app_impl(self, pkg: str, comment: CommentInfo) -> bool:
        resp = await self.client.post(
            "submit", json={"pkg_name": pkg, "comment": comment.to_json()}
        )
        if resp.status_code != 200:
            return False
        response: BaseResponse = resp.json()
        if response["success"]:
            return True
        return False

    async def submit_app(self, pkg: str, comment: Optional[CommentInfo] = None) -> bool:
        comment = comment or CommentInfo()
        comment.platform = "auto_pa"
        retry = 0
        while retry < 3:
            try:
                return await self._submit_app_impl(pkg, comment)
            except Exception as e:
                logger.traceback(f"submit app {pkg} failed: {e}")
                retry += 1
                await anyio.sleep(5)
        return False


def init_gallery(
    base_url: str,
):
    global _gallery
    _gallery = HMGallery(base_url)


def get_gallery():
    if _gallery is None:
        raise RuntimeError("gallery not initialized")
    return _gallery
