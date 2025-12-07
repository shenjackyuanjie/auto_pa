# 限制输入
from dataclasses import dataclass
import datetime
import json
import os
from pathlib import Path
from typing import Any, Optional
import anyio
from .logger import logger
from .utils import parse_log_datetime

lock = anyio.Semaphore(5)
hdc_path = os.environ.get("HDC_PATH", "hdc.exe")
_main_screen = None

@dataclass
class HDCExecProcess:
    ...

async def _exec(
    *args: str,
    timeout: Optional[float] = 30,
):
    with anyio.fail_after(
        timeout
    ):
        async with lock:
            cmd = [
                hdc_path,
                *args
            ]
            command = " ".join(cmd)
            logger.debug(f"hdc [{command}]",)
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
    res = (await _exec(
        *("shell", *args),
        timeout=timeout
    )).stdout.decode("utf-8", errors="ignore")
    return res

async def hilog(
    *args: str,
    timeout: float = 10,
) -> str:
    return await shell(*("hilog", *args), timeout=timeout)

class AdvancedProcess:
    def __init__(self, *args: str, timeout: Optional[float] = None):
        self.args = args
        self._process = None
        self._tg = None
        self.timeout = timeout

    async def __aenter__(self):
        self._tg = anyio.create_task_group()
        await self._tg.__aenter__()
        cmd = [
            hdc_path,
            *self.args
        ]
        command = " ".join(cmd)
        logger.debug(f"hdc [{command}]",)
        self._process = await anyio.open_process(
            cmd
        )
        if self.timeout is not None:
            self._tg.start_soon(self._timeout_exit)
        return self
    
    def force_exit(self):
        assert self._process is not None, "process not started"
        self._process.terminate()

    async def _timeout_exit(self):
        try:
            with anyio.fail_after(self.timeout):
                self.force_exit()
        except TimeoutError:
            pass
        except Exception as e:
            raise e
    
    async def __aexit__(self, exc_type, exc_val, exc_tb):
        assert self._process is not None, "process not started"
        assert self._tg is not None, "task group not started"
        await self._tg.__aexit__(exc_type, exc_val, exc_tb)
        self._tg = None
        
        try:
            with anyio.fail_after(10):
                self._process.kill()
                await self._process.wait()
        except TimeoutError:
            pass
        if self._process.returncode is None:  # noqa: E714
            self._process.terminate()
        self._process = None

    async def __aiter__(self):
        assert self._process is not None, "process not started"
        assert self._tg is not None, "task group not started"
        assert self._process.stdout
        while self._process is not None and self._process.returncode is None:
            async for line in self._process.stdout:
                yield line.decode("utf-8")

    def __del__(self):
        if self._process is not None and not self._process.returncode is None:  # noqa: E714
            self._process.terminate()
            self._process = None
        if self._tg is not None:
            self._tg.cancel_scope.cancel()
        logger.debug("AdvancedProcess deleted")

async def advanced_hilog(
    hilog_arg: tuple[str, ...],
    grep_arg: tuple[str, ...],
    grep_content: str,
    count: int = 1,
    timeout: Optional[float] = None,
) -> list[str]:
    # first pull last log
    start_line = (await shell(
        "hilog", *hilog_arg, "-z", "10000",
    )).splitlines()
    start_time = parse_log_datetime(start_line[-1]) if start_line else datetime.datetime.now().replace(year=1900)
    if len(start_line) == 1:
        start_time -= datetime.timedelta(seconds=1)
    res = []
    async with lock:
        async with AdvancedProcess(
            "shell",
            "hilog",
            *hilog_arg,
            "|",
            "grep",
            *grep_arg,
            grep_content,
            timeout=timeout
        ) as process:
            async for line in process:
                line = line.strip()
                print(line)
                current_time = parse_log_datetime(line)
                print(current_time, start_time)
                if current_time <= start_time or grep_content not in line:
                    continue
                print(line)
                res.append(line)
                print(res)
                if len(res) >= count:
                    print("exit")
                    break
            print(len(res))
    return res


            


async def get_main_screen_size(force: bool = False):
    global _main_screen
    if _main_screen is None or force:
        result = await shell("hidumper -s 10 -a screen")
        # find activeMode: 1080x1920, refreshRate=120
        _main_screen = tuple(map(int, result.split("activeMode: ")[1].split(",")[0].split("x")))

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

    # clean
    #await shell("rm", dump_path)

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

async def click_by_bounds(
    bounds: tuple[float, float, float, float],
    wait_for: float = 0.75
):
    await click_pos(
        (bounds[0] + bounds[2]) / 2,
        (bounds[1] + bounds[3]) / 2
    )
    await anyio.sleep(wait_for)

async def roll_to_y(
    x: float,
    y: float,
    roll_distance: float,
    wait_for: float = 1.5
):
    # main_screen = await get_main_screen_size()
    scroll = roll_distance // 15 + (1 if roll_distance % 15 != 0 else 0) # 一次，如果不满15，则向上取整
    await shell("uinput", "-M", "-m", f"{int(x)}", f"{int(y)}", "-s", f"{int(scroll) * 15}")
    await anyio.sleep(wait_for)


async def drag_to_back():
    # uinput -M -g 200 650 500 300 15000
    main_screen = await get_main_screen_size()
    from_x, to_x = 0, main_screen[0] * 0.7
    from_y, to_y = main_screen[1] * 0.75, main_screen[1] * 0.75
    await shell("uinput", "-T", "-g", "0", *map(str, map(int, [from_x, from_y, to_x, to_y])), "750", "1200")

async def reset_pointer():
    await shell("uinput", "-M", "-m", "0", "0", "-d", "0", "-u", "0")