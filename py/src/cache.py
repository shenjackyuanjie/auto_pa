from typing import Any, Optional
import diskcache


class CacheManager:
    def __init__(
        self,
        cache_dir: str = ".cache",
    ):
        self.cache = diskcache.Cache(cache_dir)

    def get(self, key: str, _default: Any = None) -> Any:
        return self.cache.get(key, _default)

    def set(self, key: str, value: Any, expire: Optional[float] = None) -> None:
        self.cache.set(key, value, expire=expire)

    def delete(self, key: str) -> None:
        self.cache.delete(key)

    def clear(self) -> None:
        self.cache.clear()


cache = CacheManager()
