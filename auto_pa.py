import json
import os
import re
import signal
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime
from pathlib import Path

import requests


# 启用 Windows 终端 ANSI 转义序列支持
if sys.platform == "win32":
    os.system("")


class TUIDisplay:
    """TUI 显示类，用于动态更新输出"""

    def __init__(self):
        self.current_line = ""

    def clear_line(self):
        """清除当前行"""
        print("\r" + " " * 100 + "\r", end="", flush=True)

    def update(self, text: str):
        """更新当前行内容（会被覆盖）"""
        self.clear_line()
        if len(text) > 95:
            text = text[:92] + "..."
        print(text, end="", flush=True)
        self.current_line = text

    def finish(self, text: str = ""):
        """完成当前行，打印永久信息并换行（不会被覆盖）"""
        self.clear_line()
        if text:
            print(text, flush=True)
        self.current_line = ""

    def log(self, text: str):
        """在保留当前状态行的情况下打印日志"""
        self.clear_line()
        print(text)
        if self.current_line:
            print(self.current_line, end="", flush=True)


tui = TUIDisplay()


class Statistics:
    def __init__(self):
        self.start_time = datetime.now()
        self.total_rounds = 0
        self.total_apps_processed = 0
        self.total_apps_shared = 0
        self.exit_reason = "未知"


stats = Statistics()


def play_beep(count: int = 3):
    """播放提示音"""
    # 尝试调用系统响铃
    for _ in range(count):
        print("\a", end="", flush=True)
        time.sleep(0.2)


def search(name: str) -> bool | None:
    """查询应用是否存在"""
    base_url = "https://hmos.txit.top/api/apps/list/1"
    params = {
        "sort": "download_count",
        "desc": "true",
        "page_size": "1",
        "search_key": "name",
        "search_value": name,
        "search_exact": "true",
    }

    try:
        回复 = requests.get(base_url, params=params, timeout=10)
        if 回复.status_code == 200:
            data = 回复.json()
            if not data.get("success"):
                return False
            if "data" not in data:
                return False
            inner_data = data["data"]
            if "data" in inner_data and len(inner_data["data"]) == 0:
                return False
            return True
    except requests.RequestException:
        return None
    return None


run_name: None | str = None


def get_layout() -> dict[str, str | list]:
    """执行 hdc shell uitest dumpLayout 获取 UI 结构"""
    tui.update("📱 正在生成 UI 结构...")

    try:
        result = subprocess.run(
            ["hdc", "shell", "uitest", "dumpLayout"],
            capture_output=True,
            text=True,
            check=True,
            encoding="utf-8",
            errors="ignore",
        )
        output = result.stdout
    except subprocess.CalledProcessError as e:
        raise RuntimeError(f"执行 dumpLayout 命令失败: {e}")
    except FileNotFoundError:
        raise RuntimeError("未找到 hdc 命令，请确保已安装并配置环境变量")

    match = re.search(r"saved to:\s*(/data/local/tmp/.*\.json)", output)
    if not match:
        raise RuntimeError(
            f"未能从输出中解析出文件路径。Output: {output.strip()[:100]}"
        )

    remote_path = match.group(1).strip()
    local_path = Path("./layout.json")

    tui.update("📥 正在拉取 layout 文件...")
    try:
        subprocess.run(
            ["hdc", "file", "recv", remote_path, str(local_path)],
            check=True,
            capture_output=True,
        )
    except subprocess.CalledProcessError as e:
        raise RuntimeError(f"拉取文件失败: {e}")

    # 清理远程文件（可选，忽略错误）
    try:
        subprocess.run(["hdc", "shell", "rm", remote_path], capture_output=True)
    except:
        pass

    try:
        data = json.loads(local_path.read_text(encoding="utf-8"))
        return data
    except Exception as e:
        raise RuntimeError(f"读取 layout.json 失败: {str(e)}")


def get_abailty(data: list[dict], name: str | None = None) -> dict | None:
    if name is None:
        return data[0] if data else None
    for item in data:
        if item.get("attributes", {}).get("abilityName") == name:
            return item
    return None


def analyze_data(data) -> list[dict]:
    tui.update("🔍 正在解析应用列表...")

    global run_name
    app_datas: list[dict] = []

    try:
        # === 按照要求替换的解析逻辑 ===
        main_child: list[dict] = data["children"]
        main_abality = get_abailty(main_child, "MainAbility")
        if main_abality is None:
            tui.finish("❌ 未找到 MainAbility")
            sys.exit(1)

        main_abality_child_1: dict = main_abality["children"][0]
        main_abality_child_2: dict = main_abality_child_1["children"][0]
        main_abality_child_3: dict = main_abality_child_2["children"][0]
        main_abality_child_4: dict = main_abality_child_3["children"][0]
        main_abality_child_5: dict = main_abality_child_4["children"][0]
        app_list_1: dict = main_abality_child_5["children"][1]
        new_app = ["新鲜应用", "新鲜游戏"]
        type_name = main_abality_child_5["children"][0]["attributes"]["text"]

        run_name = type_name

        if type_name in new_app:
            app_list_2: dict = app_list_1["children"][0]
            app_list_3: dict = app_list_2["children"][0]
            app_list_4: dict = app_list_3["children"][0]
            app_list_5: dict = app_list_4["children"][0]
            app_list_6: dict = app_list_5["children"][0]
            app_list_7: dict = app_list_6["children"][0]
            app_list_8: dict = app_list_7["children"][0]
            app_list: list[dict] = app_list_8["children"]
        else:
            app_list_2: dict = app_list_1["children"][0]
            app_list_3: dict = app_list_2["children"][0]
            app_list_4: dict = app_list_3["children"][0]
            app_list_5: dict = app_list_4["children"][0]
            app_list_6: dict = app_list_5["children"][0]
            app_list: list[dict] = app_list_6["children"]

        # 第一步：收集所有应用的基本信息
        # app_datas 已在上方初始化
        for app in app_list:
            if len(app["children"]) == 0:
                continue
            sub1 = app["children"][0]
            sub2 = sub1["children"][0]
            sub3 = sub2["children"][0]
            sub4 = sub3["children"][0]
            if len(sub4["children"]) < 4:
                # 跳过不完整 app 框
                continue
            sub5 = sub4["children"][2]
            sub6 = sub5["children"][0]
            app_name = sub6["attributes"]["text"]
            app_box: str = sub6["attributes"]["bounds"]
            # 解析 bounds 字符串格式: [x1,y1][x2,y2]
            coords = app_box.replace("[", "").replace("]", ",").split(",")
            coords = [int(coord) for coord in coords if coord]
            if len(coords) == 4:
                x1, y1, x2, y2 = coords
                center_x = (x1 + x2) // 2
                center_y = (y1 + y2) // 2
                app_datas.append(
                    {
                        "name": app_name,
                        "bounds": app_box,
                        "center": (center_x, center_y),
                        "exists": None,  # 稍后批量查询
                    }
                )
        # === 解析逻辑结束 ===

    except (IndexError, KeyError, AttributeError) as e:
        # 捕获 UI 结构不匹配导致的异常，返回空列表以便主循环重试
        tui.finish(f"⚠️ UI 解析异常: {e}")
        return []

    if not app_datas:
        return []

    # 批量查询 (保持原有逻辑)
    total_apps = len(app_datas)
    tui.update(f"🔎 正在查询应用 (0/{total_apps})...")

    completed_count = 0
    with ThreadPoolExecutor(max_workers=10) as executor:
        future_to_index = {
            executor.submit(search, app_data["name"]): idx
            for idx, app_data in enumerate(app_datas)
        }

        for future in as_completed(future_to_index):
            idx = future_to_index[future]
            try:
                found = future.result()
                app_datas[idx]["exists"] = found
            except Exception:
                app_datas[idx]["exists"] = None

            completed_count += 1
            tui.update(f"🔎 正在查询应用 ({completed_count}/{total_apps})...")

    new_apps_count = len([app for app in app_datas if app["exists"] is False])
    tui.update(f"✅ 找到 {total_apps} 个应用，其中 {new_apps_count} 个新应用")

    return app_datas


def share_at(x: int, y: int) -> None:
    # 目标点击位置
    target_pos = f"{x} {y}"
    # 构建易读的 hdc shell uinput 命令序列（保留原有执行顺序与参数）
    # 将每个参数/片段拆分成列表，便于阅读与维护
    uinput_parts = [
        "hdc",
        "shell",
        "uinput",
        "-T",
        "-d",
        target_pos,
        "-i",
        "60",
        "-u",
        target_pos,
        "-i",
        "900",
        "-d",
        "1150",
        "200",
        "-i",
        "60",
        "-u",
        "1150",
        "200",
        "-i",
        "600",
        "-d",
        "400",
        "2200",
        "-i",
        "60",
        "-u",
        "400",
        "2200",
        "-i",
        "900",
        "-d",
        "150",
        "650",
        "-i",
        "60",
        "-u",
        "150",
        "650",
        "-i",
        "400",
        "-d",
        "800",
        "1700",
        "-i",
        "60",
        "-u",
        "800",
        "1700",
        "-i",
        "300",
        "-d",
        "400",
        "2800",
        "-i",
        "60",
        "-u",
        "400",
        "2800",
        "-i",
        "300",
        "-d",
        "400",
        "2800",
        "-i",
        "60",
        "-u",
        "400",
        "2800",
    ]

    # 最终命令字符串（与原始单行命令等价）
    base_cmd = " ".join(uinput_parts)

    wait_time_ms = 3820 + 1000

    try:
        subprocess.run(base_cmd, shell=True, capture_output=True)
        time.sleep(wait_time_ms / 1000)
    except Exception as e:
        tui.finish(f"⚠️ 执行分享指令失败: {e}")


def 下滑_11() -> None:
    tui.update("📜 正在下滑页面...")
    cmd = "hdc shell uinput -M -m 500 1000 -s 2355"
    subprocess.run(cmd, shell=True, capture_output=True)
    time.sleep(1.5)


def share_app(app_datas: list[dict]) -> None:
    new_apps = [app for app in app_datas if app["exists"] is False]

    if not new_apps:
        return

    shared_names = []

    for idx, app in enumerate(new_apps, 1):
        x, y = app["center"]
        app_name = app["name"]
        shared_names.append(app_name)

        # 使用 update 动态更新，不换行
        tui.update(f"🚀 [{idx}/{len(new_apps)}] 正在分享: {app_name}")

        share_at(x, y)

    # 本轮结束后，打印一行汇总，不再覆盖，也只占一行
    tui.finish(f"✅ 本轮已分享: {', '.join(shared_names)}")


def print_statistics():
    """打印统计信息并播放结束提示音"""
    end_time = datetime.now()
    duration = end_time - stats.start_time
    hours, remainder = divmod(duration.total_seconds(), 3600)
    minutes, seconds = divmod(remainder, 60)

    tui.finish()
    print("\n" + "=" * 60)
    print("📊 运行统计信息")
    print("=" * 60)
    print(f"📂 分类名: {run_name if run_name else '未知'}")
    print(f"🔄 总运行轮次: {stats.total_rounds}")
    print(f"📱 总处理应用数: {stats.total_apps_processed}")
    print(f"🆕 总分享新应用数: {stats.total_apps_shared}")
    print(f"⏱️  运行总时长: {int(hours)}小时 {int(minutes)}分钟 {int(seconds)}秒")
    print("=" * 60)
    print(f"🚪 退出原因: {stats.exit_reason}")
    print("=" * 60)

    # 确保只要打印统计信息，就会响铃
    play_beep()


def signal_handler(signum, frame):
    """处理 Ctrl+C 信号"""
    stats.exit_reason = "用户手动停止"
    sys.exit(0)


if __name__ == "__main__":
    signal.signal(signal.SIGINT, signal_handler)

    tui.finish("🚀 程序启动...")
    tui.finish("💡 按 Ctrl+C 可随时退出并查看统计信息\n")

    previous_app_names = []

    try:
        while True:
            stats.total_rounds += 1

            data = get_layout()
            app_datas = analyze_data(data)

            if not app_datas:
                tui.update("⚠️ 本轮未获取到有效数据，尝试重新下滑...")
                下滑_11()
                continue

            current_app_names = [app["name"] for app in app_datas]

            if current_app_names == previous_app_names and len(current_app_names) > 0:
                tui.finish("🏁 检测到应用列表不再变化，任务完成")
                stats.exit_reason = "列表触底，正常结束"
                break

            stats.total_apps_processed += len(app_datas)
            new_count = len([app for app in app_datas if app["exists"] is False])
            stats.total_apps_shared += new_count

            share_app(app_datas)

            previous_app_names = current_app_names
            下滑_11()

    except SystemExit:
        pass
    except Exception as e:
        stats.exit_reason = f"程序崩溃: {str(e)}"
        tui.finish(f"❌ 发生致命错误: {e}")
    finally:
        print_statistics()
