from dataclasses import asdict, dataclass
from typing import Any, Optional, TypedDict
import weakref
import anyio
import httpx
from tianxiu2b2t.anyio.concurrency import gather

from .utils import AppInfoBuffer

from .cache import cache
from .logger import logger
import zstandard as zstd

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

class AppDetailInfo(TypedDict):
    full_info: AppInfo
    get_data: bool
    new_app: bool
    new_info: bool
    new_metric: bool
    new_rating: bool

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

@dataclass
class ShortAppInfo:
    pkg_name: str
    app_id: str
    name: str

    def __hash__(self) -> int:
        return hash(self.pkg_name)
    
@dataclass
class AllDataUrl:
    client_id: str
    url: str

class HMGallery:
    def __init__(self, base_url: str, all_data_url: Optional[AllDataUrl] = None):
        self.client = httpx.AsyncClient(base_url=base_url, http2=True)
        self.all_data_url = all_data_url
        self._all_apps: list[ShortAppInfo] = []
        self._name_mappings: weakref.WeakValueDictionary[str, ShortAppInfo] = weakref.WeakValueDictionary()
        self._pkg_mappings: weakref.WeakValueDictionary[str, ShortAppInfo] = weakref.WeakValueDictionary()
        self._app_id_mappings: weakref.WeakValueDictionary[str, ShortAppInfo] = weakref.WeakValueDictionary()

    async def get_all_data(self, force: bool = False) -> list[ShortAppInfo]:
        if self.all_data_url is None or (self._all_apps and not force):
            return self._all_apps
        
        client = httpx.AsyncClient(base_url=self.all_data_url.url, http2=True)
        resp = await client.get("token", params={
            "client_id": self.all_data_url.client_id
        })
        if resp.status_code != 200:
            logger.warning(f"get all data token failed: {resp.text}")
            return self._all_apps
        
        token = resp.json()["data"]

        resp = await client.get("get_all", params={
            "binary": True
        }, headers={
            "Authorization": f"Bearer {token}"
        })
        if resp.status_code != 200:
            logger.warning(f"get all data failed: {resp.text}")
            return self._all_apps
        content = await resp.aread()
        decompressor = zstd.ZstdDecompressor()
        decompressed = decompressor.decompressobj().decompress(content)
        buffer = AppInfoBuffer(decompressed)
        count = buffer.read_zigzag_varint()
        logger.success(f"获取到所有 App，数量 [{count}]")
        self._all_apps = [
            ShortAppInfo(
                buffer.read_string(),
                buffer.read_string(),
                buffer.read_string(),
            ) for _ in range(count)
        ]
        for app in self._all_apps:
            self._name_mappings[app.name] = app
            self._pkg_mappings[app.pkg_name] = app
            self._app_id_mappings[app.app_id] = app
        return self._all_apps

    async def init(self):
        await self.get_all_data(True)

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
        if self._name_mappings.get(name, None) is not None:
            return True
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
    ) -> dict[str, Optional[AppDetailInfo]]:
        return dict(
            zip(
                pkgs,
                await gather(
                    *(
                        self.submit_app(pkg, None, comment.clone() if comment else None)
                        for pkg in pkgs
                    )
                ),
            )
        )

    async def _submit_app_impl(self, pkg: Optional[str], app_id: Optional[str], comment: CommentInfo) -> Optional[AppDetailInfo]:
        data = {"pkg_name": pkg, "app_id": app_id, "comment": comment.to_json()}
        # del None
        data = {k: v for k, v in data.items() if v is not None}
        resp = await self.client.post(
            "submit", json=data
        )
        if resp.status_code != 200:
            return None
        response: BaseResponse = resp.json()
        if response["success"]:
            return response["data"]
        return None

    async def submit_app(self, pkg: Optional[str], app_id: Optional[str], comment: Optional[CommentInfo] = None) -> Optional[AppDetailInfo]:
        comment = comment or CommentInfo()
        comment.platform = "auto_pa"
        retry = 0
        while retry < 3:
            try:
                return await self._submit_app_impl(pkg, app_id, comment)
            except Exception as e:
                logger.traceback(f"submit app {pkg or app_id} failed: {e}")
                retry += 1
                await anyio.sleep(5)
        return None

    @property
    def all_apps(self) -> list[ShortAppInfo]:
        return list(self._all_apps)

    @property
    def pkg_apps(self) -> dict[str, ShortAppInfo]:
        return dict(self._pkg_mappings)
    
    @property
    def app_ids(self) -> dict[str, ShortAppInfo]:
        return dict(self._app_id_mappings)
    
    @property
    def app_names(self) -> dict[str, ShortAppInfo]:
        return dict(self._name_mappings)
    
    async def exists_app(
        self,
        data: str
    ) -> Optional[ShortAppInfo]:
        await self.get_all_data()
        info = None
        if data in self.pkg_apps:
            info = self.pkg_apps[data]
        elif data in self.app_ids:
            info = self.app_ids[data]
        elif data in self.app_names:
            info = self.app_names[data]
        return info

async def init_gallery(
    base_url: str,
    all_data_url: Optional[AllDataUrl] = None
):
    global _gallery
    _gallery = HMGallery(base_url, all_data_url)
    await _gallery.init()


def get_gallery():
    if _gallery is None:
        raise RuntimeError("gallery not initialized")
    return _gallery
