from dataclasses import dataclass, field
import datetime
import re
from typing import Any, List

JSON_PATH = List[str | int]


@dataclass
class LinksPkgsAppIds:
    links: List[str] = field(default_factory=list)
    pkgs: List[str] = field(default_factory=list)
    app_ids: List[str] = field(default_factory=list)

    def empty(self):
        return len(self.links) == 0 and len(self.pkgs) == 0 and len(self.app_ids) == 0


def find_json_value_as_path(data: Any, value: Any) -> List[JSON_PATH]:
    """
    在嵌套的字典/列表中，查找所有值与 `value` 相等的完整路径。
    路径格式为 `JSON_PATH` 类型，例如：`['store', 'book', 1, 'price']` 代表 `$.store.book[1].price`
    """
    return regex_json_value_as_path(data, value)


def regex_json_value_as_path(data: Any, value: re.Pattern | Any):
    result: List[JSON_PATH] = []

    if isinstance(data, dict):
        for key, val in data.items():
            # 1. 如果当前值匹配，则将当前键作为路径加入结果
            if (
                isinstance(value, re.Pattern)
                and isinstance(val, (str, int, float, bool))
                and value.fullmatch(str(val)) is not None
            ):
                result.append([key])
            elif val == value:
                result.append([key])
            # 2. 如果是嵌套结构，递归查找，并将当前键添加到子路径的开头
            elif isinstance(val, (dict, list)):
                for sub_path in find_json_value_as_path(val, value):
                    result.append([key] + sub_path)  # 关键：路径拼接

    elif isinstance(data, list):
        for idx, val in enumerate(data):
            # 1. 如果当前值匹配，则将当前索引作为路径加入结果
            if (
                isinstance(value, re.Pattern)
                and isinstance(val, (str, int, float, bool))
                and value.fullmatch(str(val)) is not None
            ):
                result.append([idx])
            elif val == value:
                result.append([idx])
            # 2. 如果是嵌套结构，递归查找，并将当前索引添加到子路径的开头
            elif isinstance(val, (dict, list)):
                for sub_path in find_json_value_as_path(val, value):
                    result.append([idx] + sub_path)  # 关键：路径拼接

    return result


def find_json_value_by_path(
    data: Any, path: JSON_PATH, raise_error: bool = False
) -> Any:
    """
    根据给定的路径（`JSON_PATH` 类型）在数据中查找对应的值。
    """
    current = data
    for key in path:
        try:
            current = current[key]  # key 可以是 str (字典) 或 int (列表)
        except KeyError:
            if raise_error:
                raise
            return None
    return current


def find_json_value_by_prev_path(data: Any, path: JSON_PATH, deep: int = 1) -> Any:
    """
    根据给定的路径（`JSON_PATH` 类型）在数据中查找对应的值。
    """
    # pop 最后一个元link
    prev_path = path[:-deep]
    return find_json_value_by_path(data, prev_path)


def list_json_value_by_paths(data: Any, path: List[JSON_PATH]) -> List[Any]:
    """
    根据给定的路径（`JSON_PATH` 类型）在数据中查找对应的值。
    """
    return [find_json_value_by_path(data, p) for p in path]


def list_json_value_by_prev_paths(
    data: Any, path: List[JSON_PATH], deep: int = 1
) -> List[Any]:
    """
    根据给定的路径（`JSON_PATH` 类型）在数据中查找对应的值。
    """
    return [find_json_value_by_prev_path(data, p, deep=deep) for p in path]


def parse_bounds(bounds: str) -> tuple[int, int, int, int]:
    """
    解析 bounds 字符串，例如：`[0,0][1080,1920]`，返回 (x1, y1, x2, y2)
    """
    first, last = bounds.split("][")
    x1, y1 = map(int, first[1:].split(","))
    x2, y2 = map(int, last[:-1].split(","))
    return x1, y1, x2, y2


def is_in_area(input_bounds: str, area_bounds: str, tolerance: int = 0) -> bool:
    """
    判断输入的 bounds 是否在指定的 area_bounds 内
    """
    x1, y1, x2, y2 = parse_bounds(input_bounds)
    ax1, ay1, ax2, ay2 = parse_bounds(area_bounds)

    return (
        x1 >= ax1 - tolerance
        and y1 >= ay1 - tolerance
        and x2 <= ax2 + tolerance
        and y2 <= ay2 + tolerance
    )


def json_dumps(
    data: Any,
    indent: int = 4,
    ensure_ascii: bool = False,
    sort_keys: bool = True,
) -> str:
    import json

    return json.dumps(
        data,
        indent=indent,
        ensure_ascii=ensure_ascii,
        sort_keys=sort_keys,
    )


def parse_input_split_links_pkgs_and_app_ids(input_str: str) -> LinksPkgsAppIds:
    """
    解析输入字符串，提取链接、包名和app_id

    Args:
        input_str: 输入的字符串

    Returns:
        包含links, pkgs, app_ids的字典
    """
    input_str = input_str.strip()
    if not input_str:
        return LinksPkgsAppIds()

    # 支持更多分隔符：空格、换行、逗号、分号、竖线等
    parts = re.split(r"[\s\n,;|]+", input_str)
    url_like = re.compile(r"^https?://[^\s]+$")

    # 修改正则，将 C+数字 和其他包名分开匹配
    app_id_regex = re.compile(r"^[Cc]\d+$")  # 匹配 C 开头的数字（app_id）
    pkg_name_regex = re.compile(
        r"^[a-zA-Z][a-zA-Z0-9_]*(\.[a-zA-Z0-9_]+)+$"
    )  # 匹配传统包名

    # 用于从文本中提取的正则
    extract_pkg_regex = re.compile(r"([a-zA-Z][a-zA-Z0-9_]*(\.[a-zA-Z0-9_]+)+)")
    extract_app_id_regex = re.compile(r"[Cc]\d+")

    links: List[str] = []
    pkgs: List[str] = []
    app_ids: List[str] = []
    for part in parts:
        start = 0
        # first app_id / pkg and then url
        while start < len(part):
            # 匹配 app_id
            match = app_id_regex.search(part, start)
            if match:
                app_ids.append(match.group())
                start = match.end()
                continue

            # 匹配 pkg_name
            match = pkg_name_regex.search(part, start)
            if match:
                pkgs.append(match.group())
                start = match.end()
                continue

            # 匹配 url
            match = url_like.search(part, start)
            if match:
                # 匹配包名 / app_id
                content = match.group()
                links.append(content)
                ls = 0
                while ls < len(content):
                    m = extract_pkg_regex.search(content, ls)
                    if m:
                        pkgs.append(m.group())
                        ls = m.end()
                        continue

                    m = extract_app_id_regex.search(content, ls)
                    if m:
                        app_ids.append(m.group())
                        ls = m.end()
                        continue
                    ls += 1
                start = match.end()
                continue

            # 匹配其他包名
            match = extract_pkg_regex.search(part, start)
            if match:
                pkgs.append(match.group())
                start = match.end()
                continue

            # 匹配其他 app_id
            match = extract_app_id_regex.search(part, start)
            if match:
                app_ids.append(match.group())
                start = match.end()
                continue

            # 如果没有匹配到，则跳过一个字符
            start += 1

    # 按顺序的去重，也就是根据输入的顺序
    final_links = []
    final_pkgs = []
    final_app_ids = []
    for item in links:
        if item in final_links:
            continue
        final_links.append(item)

    for item in pkgs:
        if item in final_pkgs:
            continue
        final_pkgs.append(item)

    for item in app_ids:
        if item in final_app_ids:
            continue
        final_app_ids.append(item)

    return LinksPkgsAppIds(
        links=final_links,
        pkgs=final_pkgs,
        app_ids=final_app_ids,
    )


def parse_log_datetime(log: str) -> datetime.datetime:
    # now = datetime.datetime.now()
    date, time, _ = log.split(" ", 2)
    # 12-07 16:01:30.000
    return datetime.datetime.strptime(f"{date} {time}", "%m-%d %H:%M:%S.%f")
