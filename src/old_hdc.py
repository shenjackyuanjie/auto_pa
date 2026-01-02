# 限制输入
import asyncio
from dataclasses import dataclass
import json
import os
from pathlib import Path
from typing import Any, Optional
import anyio

from . import utils
from .logger import logger

lock = anyio.Semaphore(5)
hdc_path = os.environ.get("HDC_PATH", "hdc.exe")
_main_screen = None


@dataclass
class Ethernet:
    type: str
    name: str
    ip: Optional[str] = None
    mac: Optional[str] = None


class HilogProcess:
    def __init__(self, *args: str):
        self._args = args
        self._process = None

    async def run_forever(self):
        async with self:
            assert self._process is not None, "process not started"
            await self._process.wait()

    async def __aenter__(self):
        cmd = [hdc_path, "shell", "hilog", *self._args]
        command = " ".join(cmd)
        logger.debug(f"hdc [{command}]")

        self._process = await asyncio.create_subprocess_exec(
            *cmd,
            stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.DEVNULL,
        )
        return self

    def force_exit(self):
        if self._process and self._process.returncode is None:
            self._process.kill()

    async def exit(self):
        if self._process is None or self._process.returncode is not None:
            return
        self._process.terminate()
        await self._process.wait()

    async def __aexit__(self, exc_type, exc_val, exc_tb):
        if self._process is None:
            return

        # 关键修改：不再使用 wait_for 包裹整个退出逻辑。
        # 这样，当外部发送 CancelledError (如 Ctrl+C) 时，它会被直接传播出去，不会被捕获。
        try:
            # 尝试优雅退出，但设置一个简单的超时后台任务
            await asyncio.wait_for(self.exit(), timeout=10)
        except asyncio.TimeoutError:
            # 仅捕获超时异常，不捕获 CancelledError
            logger.warning("Process exit timed out, forcing kill.")
            self.force_exit()
            # 即使强制终止，也等待一下避免僵尸进程
            await self._process.wait()
        except asyncio.CancelledError:
            # 如果收到取消信号（如 Ctrl+C），先强制杀死子进程，然后重新抛出该异常
            logger.info("Received cancellation, killing subprocess.")
            self.force_exit()
            # 不等待进程结束，立即重新抛出取消异常，允许上层处理
            raise
        finally:
            # 无论是否发生异常，都确保清理进程引用
            self._process = None
            logger.debug("Hilog Process cleaned up.")

    @property
    def _stdout(self):
        if self._process is None:
            raise RuntimeError("Process not started")
        return self._process.stdout

    async def readline(self, encoding: str = "utf-8", errors: str = "ignore") -> str:
        if self._stdout is None:
            raise RuntimeError("Stdout is not available")
        line_bytes = await self._stdout.readline()
        return line_bytes.decode(encoding, errors=errors).rstrip("\n")

    async def __aiter__(self):
        if self._stdout is None:
            return
        while True:
            line = await self.readline()
            if not line:
                break
            yield line

    def __del__(self):
        if self._process is not None and self._process.returncode is None:
            try:
                self._process.terminate()
            except Exception as e:
                logger.debug(
                    f"Ignored error during process termination in __del__: {e}"
                )
        logger.debug("HilogProcess instance deleted.")


async def _exec(
    *args: str,
    timeout: Optional[float] = 30,
):
    with anyio.fail_after(timeout):
        async with lock:
            cmd = [hdc_path, *args]
            command = " ".join(cmd)
            logger.debug(
                f"hdc [{command}]",
            )
            res = await anyio.run_process(
                cmd,
            )
        if res.returncode != 0:
            raise RuntimeError(res.stderr.decode("utf-8"))
        return res


async def shell(
    *args: str,
    timeout: float = 10,
) -> str:
    # quote_args = (shlex.quote(arg) for arg in args)
    res = (await _exec(*("shell", *args), timeout=timeout)).stdout.decode(
        "utf-8", errors="ignore"
    )
    return res


async def get_device_type(
    timeout: float = 10,
) -> str:
    return (
        await shell(*("param", "get", "const.product.devicetype"), timeout=timeout)
    ).strip()


async def get_main_screen_size(force: bool = False):
    global _main_screen
    if _main_screen is None or force:
        result = await shell("hidumper -s 10 -a screen")
        # find activeMode: 1080x1920, refreshRate=120
        _main_screen = tuple(
            map(int, result.split("activeMode: ")[1].split(",")[0].split("x"))
        )

    return _main_screen


async def is_phone_mode():
    size = await get_main_screen_size()
    return size[0] < size[1]


async def dump_layout_to_text() -> str:
    res = await shell("uitest", "dumpLayout")
    try:
        dump_path = res.split("saved to:")[1].strip()
    except Exception as e:
        raise RuntimeError("dumpLayout 命令执行失败") from e

    res = await shell("cat", dump_path)

    return res


async def dump_layout_to_json() -> Any:
    res = await dump_layout_to_text()
    try:
        return json.loads(res)
    except Exception as e:
        raise RuntimeError("解析布局文件失败") from e


async def dump_layout_to_file(output: Path, use_json: bool = True) -> Any:
    res = await dump_layout_to_text()
    with open(output, "w", encoding="utf-8") as f:
        if use_json:
            json.dump(json.loads(res), f, indent=4, ensure_ascii=False)
        else:
            f.write(res)


async def click_pos(
    x: float,
    y: float,
):
    await shell("uinput", "-M", "-m", f"{int(x)}", f"{int(y)}", "-d", "0", "-u", "0")

async def click_pos_by_scale(
    x_scale: float,
    y_scale: float,
):
    main_screen = await get_main_screen_size()
    await click_pos(main_screen[0] * x_scale, main_screen[1] * y_scale)

async def click_by_bounds(
    bounds: tuple[float, float, float, float] | str, wait_for: float = 0.75
):
    bounds = utils.parse_bounds(bounds) if isinstance(bounds, str) else bounds
    await click_pos((bounds[0] + bounds[2]) / 2, (bounds[1] + bounds[3]) / 2)
    await anyio.sleep(wait_for)


async def roll_to_y(
    x_scale: float, y_scale: float, roll_distance: float, wait_for: float = 1.5
):
    main_screen = await get_main_screen_size()
    scroll = roll_distance // 15 + (
        1 if roll_distance % 15 != 0 else 0
    )  # 一次，如果不满15，则向上取整
    await shell(
        "uinput",
        "-M",
        "-m",
        f"{int(main_screen[0] * x_scale)}",
        f"{int(main_screen[1] * y_scale)}",
        "-s",
        f"{int(scroll) * 15}",
    )
    await anyio.sleep(wait_for)


async def simple_roll_down(
    x_scale: float, y_scale: float, roll_scale: float, wait_for: float = 1.5
):
    main_screen = await get_main_screen_size()
    await roll_to_y(x_scale, y_scale, main_screen[1] * roll_scale, wait_for)


async def drag_to_back():
    # uinput -M -g 200 650 500 300 15000
    main_screen = await get_main_screen_size()
    from_x, to_x = 0, main_screen[0] * 0.7
    from_y, to_y = main_screen[1] * 0.75, main_screen[1] * 0.75
    await shell(
        "uinput",
        "-T",
        "-g",
        "0",
        *map(str, map(int, [from_x, from_y, to_x, to_y])),
        "750",
        "1200",
    )


async def reset_pointer():
    await shell("uinput", "-M", "-m", "0", "0", "-d", "0", "-u", "0")
