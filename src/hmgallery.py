from dataclasses import dataclass
from typing import Any, Optional, TypedDict
import httpx
from tianxiu2b2t.anyio.concurrency import gather

from .cache import cache 
from .logger import logger

_gallery: Optional['HMGallery'] = None

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

class HMGallery:
    def __init__(self, base_url: str):
        self.client = httpx.AsyncClient(base_url=base_url, http2=True)

    async def search_app_names_exists(
        self,
        *names: str,
    ) -> dict[str, bool]:
        return dict(zip(names, await gather(*(self.search_app_name_exists(name) for name in names))))
        

    async def search_app_name_exists(
        self,
        name: str,
    ) -> bool:
        if cache.get(f"app:{name}", False) is True:
            logger.debug(f"[{name}] is exists")
            return True
        idx = 0
        params = {
            "sort": "download_count",
            "desc": True,
            "page_size": 50,
            "search_key": "name",
            "search_value": name,
            "search_exact": True,
        }
        while 1:
            resp = await self.client.get(f"apps/list/{(idx := idx + 1)}", params=params)
            if resp.status_code == 200:
                response: BaseResponse = resp.json()
                data: SearchListResponse = response["data"]
                # find name
                for app in data["data"]:
                    if app["name"].lower() == name.lower():
                        cache.set(f"app:{name}", True)
                        return True
                if data["total_pages"] <= idx:
                    break

        return False

    async def submit_apps(
        self,
        *pkgs: str,
        comment: Optional[CommentInfo] = None
    ) -> dict[str, bool]:
        return dict(zip(pkgs, await gather(*(self.submit_app(pkg, comment) for pkg in pkgs))))
    
    async def submit_app(
        self,
        pkg: str,
        comment: Optional[CommentInfo] = None
    ) -> bool:
        comment = (comment or CommentInfo())
        comment.platform = "auto_pa"
        resp = await self.client.post("submit", json={
            "pkg_name": pkg,
            "comment": comment
        })
        return resp.status_code == 200

def init_gallery(
    base_url: str,
):
    global _gallery
    _gallery = HMGallery(base_url)


def get_gallery():
    if _gallery is None:
        raise RuntimeError("gallery not initialized")
    return _gallery