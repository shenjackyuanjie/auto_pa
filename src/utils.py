import re
from typing import Any, List, Tuple

JSON_PATH = List[str | int]
LINKS_AND_PKGS = Tuple[List[str], List[str]]

def find_json_value_as_path(data: Any, value: Any) -> List[JSON_PATH]:
    """
    在嵌套的字典/列表中，查找所有值与 `value` 相等的完整路径。
    路径格式为 `JSON_PATH` 类型，例如：`['store', 'book', 1, 'price']` 代表 `$.store.book[1].price`
    """
    result: List[JSON_PATH] = []

    if isinstance(data, dict):
        for key, val in data.items():
            # 1. 如果当前值匹配，则将当前键作为路径加入结果
            if val == value:
                result.append([key])
            # 2. 如果是嵌套结构，递归查找，并将当前键添加到子路径的开头
            elif isinstance(val, (dict, list)):
                for sub_path in find_json_value_as_path(val, value):
                    result.append([key] + sub_path)  # 关键：路径拼接

    elif isinstance(data, list):
        for idx, val in enumerate(data):
            # 1. 如果当前值匹配，则将当前索引作为路径加入结果
            if val == value:
                result.append([idx])
            # 2. 如果是嵌套结构，递归查找，并将当前索引添加到子路径的开头
            elif isinstance(val, (dict, list)):
                for sub_path in find_json_value_as_path(val, value):
                    result.append([idx] + sub_path)  # 关键：路径拼接

    return result

def find_json_value_by_path(data: Any, path: JSON_PATH, raise_error: bool = False) -> Any:
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

def list_json_value_by_prev_paths(data: Any, path: List[JSON_PATH], deep: int = 1) -> List[Any]:
    """
    根据给定的路径（`JSON_PATH` 类型）在数据中查找对应的值。
    """
    return [find_json_value_by_prev_path(data, p, deep=deep) for p in path]


def parse_bounds(bounds: str) -> tuple[int, int, int, int]:
    """
    解析 bounds 字符串，例如：`[0,0][1080,1920]`，返回 (x1, y1, x2, y2)
    """
    first, last = bounds.split('][')
    x1, y1 = map(int, first[1:].split(','))
    x2, y2 = map(int, last[:-1].split(','))
    return x1, y1, x2, y2

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


def parse_input_split_links_and_pkgs_tuple(input_text: str) -> LINKS_AND_PKGS:
    """
    返回元组而不是字典（更Pythonic的方式）
    """
    if not input_text or not input_text.strip():
        return [], []
    
    parts = re.split(r'[\s\n]+', input_text.strip())
    url_like = re.compile(r'^https?://[^\s]+$')
    pkg_regex = re.compile(r'^[a-zA-Z][a-zA-Z0-9_]*\.[a-zA-Z0-9_.]+$')
    
    links = []
    pkgs = []
    seen_pkgs = set()  # 使用集合来跟踪已添加的包名以提高性能
    
    for part in parts:
        if url_like.match(part):
            links.append(part)
            
            id_match = re.search(r'id=([a-zA-Z][a-zA-Z0-9_]*\.[a-zA-Z0-9_.]+)', part)
            if id_match:
                pkg = id_match.group(1)
                if pkg not in seen_pkgs:
                    pkgs.append(pkg)
                    seen_pkgs.add(pkg)
                    
        elif pkg_regex.match(part):
            if part not in seen_pkgs:
                pkgs.append(part)
                seen_pkgs.add(part)
    
    return links, pkgs