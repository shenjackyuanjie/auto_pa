from dataclasses import asdict, dataclass
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

    def clone(self):
        return CommentInfo(**asdict(self))


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
            stable_count = 0
            while (stable_count := stable_count + 1) < 3:
                current_idx = (idx := idx + 1)
                resp = await self.client.get(f"apps/list/{current_idx}", params=params)
                if resp.status_code == 200:
                    response: BaseResponse = resp.json()
                    data: SearchListResponse = response["data"]
                    # find name
                    for app in data["data"]:
                        if app["name"].lower() == name.lower():
                            cache.set(f"app:{name}", True, 3600)
                            return True
                    if data["total_pages"] <= current_idx:
                        return False

        return False

    async def submit_apps(
        self,
        *pkgs: str,
        comment: Optional[CommentInfo] = None
    ) -> dict[str, bool]:
        return dict(zip(pkgs, await gather(*(self.submit_app(pkg, comment.clone() if comment else None) for pkg in pkgs))))
    
    async def submit_app(
        self,
        pkg: str,
        comment: Optional[CommentInfo] = None
    ) -> bool:
        comment = (comment or CommentInfo())
        comment.platform = "auto_pa"
        resp = await self.client.post("submit", json={
            "pkg_name": pkg,
            "comment": asdict(comment)
        })
        if resp.status_code != 200:
            return False
        response: BaseResponse = resp.json()
        if response["success"]:
            return True
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