import asyncio
from dataclasses import dataclass
import datetime
import json
import os
from typing import Any, Optional

import anyio
from .logger import logger
from tianxiu2b2t.anyio.concurrency import gather
from tianxiu2b2t.utils import runtime
from src import utils

hdc_path = os.environ.get("HDC_PATH", "hdc.exe")
DEFAULT_TIMEOUT = 30

@dataclass
class AppInfo:
    version_code: int
    version_name: str
    update_time: datetime.datetime


@dataclass
class DeviceInfo:
    name: str
    main_screen: tuple[int, int]
    device_type: str
    model: str
    sn: str

    @staticmethod
    async def init(device: 'Device'):
        parameters = [
            f"param get {x}" for x in (
                "const.product.devicetype",
                "const.product.model",
                "const.product.name"
            )
        ] + [
            "echo '\t\t'",
            "SP_daemon -deviceinfo"
        ]
        
        (param, deviceinfo) = (line.strip() for line in (await device.shell("; ".join(parameters))).split("\t\t", 1))
        device_type, model, name = (line.strip() for line in param.splitlines())
        sn = DeviceInfo._find_value(deviceinfo, "sn")
        main_screen: tuple[int, int] = tuple(map(int, DeviceInfo._find_value(deviceinfo, "activeMode").split("x", 1))) # type: ignore
        return DeviceInfo(name, main_screen, device_type, model, sn)

    
    @staticmethod
    def _find_value(data: str, key: str) -> str:
        for line in data.splitlines():
            if line.startswith(key):
                return line.split(":")[1].strip()
        return ""
        

class Device:
    def __init__(self, device: str, connection_type: str):
        self._device = device
        self._connection_type = connection_type.lower()
        self._device_info = None
        self._bottom_bar: Optional[str] = None

    def __repr__(self) -> str:
        assert self._device_info is not None
        return f"Device({self.display_device_id}, {self.connection_type}, {self.device_type}, {self.model}, {self.name})"
    
    @property
    def tag(self):
        return f"{self.name} ({self.model}, {self.display_device_id})"

    @property
    def display_device_id(self):
        # if self.connection_type == "usb":
        if self.connection_type == "usb":
            total = len(self.device_id)
            start = 3
            end = max(total - 3, start)
            return self.device_id[:start] + ("*" * (total - (end - start))) + self.device_id[end:]
        return self.device_id
    @property
    def device_id(self) -> str:
        return self._device
    
    @property
    def connection_type(self) -> str:
        return self._connection_type

    @property
    def name(self) -> str:
        assert self._device_info is not None
        return self._device_info.name
    
    @property
    def main_screen(self) -> tuple[int, int]:
        assert self._device_info is not None
        return self._device_info.main_screen

    @property
    def device_type(self) -> str:
        assert self._device_info is not None
        return self._device_info.device_type
    
    @property
    def model(self) -> str:
        assert self._device_info is not None
        return self._device_info.model

    @property
    def sn(self) -> str:
        assert self._device_info is not None
        return self._device_info.sn
        

    async def init(self):
        """这里用来初始化一下默认的东西，方便下次快速拿取"""
        # bingfa
        self._device_info = await DeviceInfo.init(self)
        return self
    
    async def shell(
        self,
        *args: str,
        timeout: float = DEFAULT_TIMEOUT,
    ) -> str:
        # quote_args = (shlex.quote(arg) for arg in args)
        res = (await _exec(*("shell", *args), timeout=timeout, device=self.device_id)).stdout.decode(
            "utf-8", errors="ignore"
        )
        return res
            
    async def get_ping(self):
        if self.connection_type != "tcp":
            return -1
        try:
            host, port = self.device_id.rsplit(":", 1)
            start_time = runtime.perf_counter_ns()
            r, w = (await asyncio.open_connection(host, int(port)))
            w.close()
            await w.wait_closed()
            end_time = runtime.perf_counter_ns()
            return (end_time - start_time) / 1e6
        except Exception:
            logger.traceback()
            return -1
    
    async def _dump_layout_to_text(self) -> str:
        res = await self.shell("export DUMPLAYOUT_TMP=$(uitest dumpLayout | cut -d ':' -f2-); cat $DUMPLAYOUT_TMP; rm $DUMPLAYOUT_TMP")
        return res

    async def dump_layout_to_json(self, fuck_usb_connection: bool = True) -> Any:
        res = await self._dump_layout_to_text()
        try:
            result = json.loads(res)
            path = utils.find_json_value_as_path(result, "USB 连接方式")
            if not fuck_usb_connection or not path:
                return result
            await self.click_pos_by_scale(0.5, 0.8)
            return await self.dump_layout_to_json()
        except Exception:
            logger.traceback()
            return res
        
    async def click_pos(
        self,
        x: float,
        y: float,
    ):
        await self.shell("uinput", "-M", "-m", f"{int(x)}", f"{int(y)}", "-d", "0", "-u", "0")

    async def click_pos_by_scale(
        self,
        x_scale: float,
        y_scale: float,
    ):
        main_screen = self.main_screen
        await self.click_pos(main_screen[0] * x_scale, main_screen[1] * y_scale)

    async def click_by_bounds(
            self,
        bounds: tuple[float, float, float, float] | str, wait_for: float = 0.75
    ):
        bounds = utils.parse_bounds(bounds) if isinstance(bounds, str) else bounds
        await self.click_pos((bounds[0] + bounds[2]) / 2, (bounds[1] + bounds[3]) / 2)
        await anyio.sleep(wait_for)

    async def roll_to_y(
            self,
        x_scale: float, y_scale: float, roll_distance: float, wait_for: float = 1.5
    ):
        main_screen = self.main_screen
        scroll = roll_distance // 15 + (
            1 if roll_distance % 15 != 0 else 0
        )  # 一次，如果不满15，则向上取整
        await self.shell(
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
            self,
        x_scale: float, y_scale: float, roll_scale: float, wait_for: float = 1.5
    ):
        main_screen = self.main_screen
        await self.roll_to_y(x_scale, y_scale, main_screen[1] * roll_scale, wait_for)

    async def drag_to_back(self,):
        # uinput -M -g 200 650 500 300 15000
        main_screen = self.main_screen
        from_x, to_x = 0, main_screen[0] * 0.7
        from_y, to_y = main_screen[1] * 0.75, main_screen[1] * 0.75
        await self.shell(
            "uinput",
            "-T",
            "-g",
            "0",
            *map(str, map(int, [from_x, from_y, to_x, to_y])),
            "750",
            "1200",
        )

    async def reset_pointer(self,):
        await self.shell("uinput", "-M", "-m", "0", "0", "-d", "0", "-u", "0")

    async def open_app(self, package: str, ability: str):
        await self.shell("aa", "start", "-a", ability, "-b", package)
        
    async def close_app(self, package: str):
        await self.shell("aa", "force-stop", package)

    async def get_bottom_bar(self):
        if self._bottom_bar is not None:
            return self._bottom_bar
        layout = await self.dump_layout_to_json()
        # com.huawei.hms.floatingnavigation
        paths = utils.find_json_value_as_path(layout, "com.huawei.hms.floatingnavigation")
        if not paths:
            pos = (
                self.main_screen[0] // 3,
                int(self.main_screen[1] * 0.95),
            )
            size = (
                self.main_screen[0] // 3,
                int(self.main_screen[1] * 0.05)
            )
            self._bottom_bar = f"[{pos[0]},{pos[1]}][{pos[0] + size[0]},{pos[1] + size[1]}]"
            return self._bottom_bar
        self._bottom_bar = utils.find_json_value_by_prev_path(layout, paths[0])["bounds"]
        assert self._bottom_bar is not None, "bottom bar not found"
        return self._bottom_bar

    async def get_app_info(self, package: str) -> Optional[AppInfo]:
        res = (await self.shell("bm", "dump", "-n", package)).strip()
        if res.startswith("error"):
            return None
        res = json.loads(res.strip(f"{package}:").strip())
        return AppInfo(
            version_code=res["versionCode"],
            version_name=res["versionName"],
            update_time=datetime.datetime.fromtimestamp(res["updateTime"] / 1000.0),
        )


_devices: dict[str, Device] = {}

class HilogProcess:
    def __init__(self, device_id: str, *args: str):
        self.device_id = device_id
        self._args = args
        self._process = None

    async def run_forever(self):
        async with self:
            assert self._process is not None, "process not started"
            await self._process.wait()

    async def __aenter__(self):
        cmd = [hdc_path, "-t", self.device_id, "shell", "hilog", *self._args]
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
        except Exception:
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
    device: Optional[str] = None,
    timeout: Optional[float] = 30,
):
    with anyio.fail_after(timeout):
        d = ["-t", device] if device is not None else []
        cmd = [hdc_path, *d, *args]
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
    device: str,
    timeout: float = DEFAULT_TIMEOUT,
) -> str:
    # quote_args = (shlex.quote(arg) for arg in args)
    res = (await _exec(*("shell", *args), timeout=timeout, device=device)).stdout.decode(
        "utf-8", errors="ignore"
    )
    return res

async def _get_sn(
    device: str,
):
    res = (await shell("SP_daemon", "-deviceinfo", timeout=DEFAULT_TIMEOUT, device=device)).splitlines()
    """sn: """
    # find it
    for line in res:
        if line.startswith("sn: "):
            return line.split("sn: ")[1].strip()
    raise RuntimeError("can not find sn")



async def refresh_targets():
    res = (await _exec("list", "targets", "-v")).stdout.decode("utf-8")
    devices = sorted(
        [(line.split("\t\t", 1)[0], line.split("\t\t", 1)[1].split("\t", 1)[0]) for line in res.strip().splitlines() if line.strip() and 'Connected' in line],
        key=lambda x: x[1],
    )
    sns = await gather(
        *[_get_sn(device) for device, _ in devices]
    )
    # 如果不存在则添加，如果devices没有的话，_devices有则删除
    new_devices = []
    for (device, connection_type), sn in zip(devices, sns):
        if device not in _devices:
            _devices[sn] = Device(device, connection_type=connection_type)
            new_devices.append(sn)
    
    # init 
    await gather(
        *[_devices[sn].init() for sn in new_devices]
    )

    
    for device in list(_devices.keys()):
        if device not in sns:
            del _devices[device]

async def get_targets():
    await refresh_targets()
    return _devices

async def get_devices():
    await refresh_targets()
    return list(_devices.values())

async def get_device(sn: str):
    await refresh_targets()
    return _devices[sn]
