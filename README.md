<div align="center">

# Auto_pa

</div>

Rust 版本只有一个入口，使用 `search` 和 `hilog` 子命令：

```powershell
cargo run --release -- <子命令> [参数]
```

## 目录结构

```text
auto_pa.py / main.py / search.py  保留在根目录的 Python 入口
py/core/                          Python 业务流程
py/src/                           Python 运行时、HDC 与 API 支持代码
src/                              Rust 的 Cargo 默认源目录
```

## Rust UI 搜索

基于 `hm_driver_rs` 实现的 AppGallery UI 搜索流程：

```powershell
cargo run --release -- search --fresh
```

原有 Python 入口仍可使用：

```powershell
python search.py --fresh
```

Rust 默认输出 `INFO` 日志到终端和
`logs/search-rust.log.YYYY-MM-DD`。使用 `-v` 输出 `DEBUG` 日志，使用
`--disable-log-file` 禁用日志文件。

当前待搜索名称完成后，程序会再遍历一次分类，只搜索新发现的名称。每台设备的
进度保存在 `.cache/search/` 下。

## Rust UI 分类遍历

`hilog` 子命令实现了 Python 命令
`uv run .\\main.py hilog --no-submit` 的纯 UI 部分。它会在每台在线设备上启动
AppGallery，遍历「应用」和「游戏」的分类页面，并将每个应用列表下滑至稳定。
默认不抓取 hilog，也不提交应用：

```powershell
cargo run --release -- hilog
```

可用参数包括 `--skip-categories <名称>...`、`--loop <次数>`、
`--loop-wait 5m`、`--ping 15` 和 `--keep-open-on-error`。`--submit`
为未来的 hilog 抓取和应用投稿流程预留；当前版本会明确拒绝该参数，避免悄悄执行
不完整的投稿流程。
