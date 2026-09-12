<div align="center">

# Auto_pa

</div>

Rust 版本只有一个入口，使用 `search` 和 `hilog` 子命令：

```powershell
cargo run --release -- <子命令> [参数]
```

Python 版本（`auto_pa.py`、`main.py`、`search.py` 与 `py/`）已经废弃，不再维护，
也不会同步新功能，请统一使用 Rust 入口。

## 目录结构

```text
src/                              Rust 源码
docs/                             架构与开发文档
auto_pa.py / main.py / search.py  已废弃的 Python 入口
py/                               已废弃的 Python 业务流程与运行时
```

## Rust UI 搜索

基于 `hm_driver_rs` 实现的 AppGallery UI 搜索流程：

```powershell
cargo run --release -- search --fresh
```

Rust 默认输出 `INFO` 日志到终端和
`logs/search-rust.log.YYYY-MM-DD`。使用 `-v` 输出 `DEBUG` 日志，使用
`--disable-log-file` 禁用日志文件。

当前待搜索名称完成后，程序会再遍历一次分类，只搜索新发现的名称。每台设备的
进度保存在 `.cache/search/` 下。

可用参数：

| 参数 | 说明 |
| --- | --- |
| `--fresh` | 丢弃已保存的进度，从头开始 |
| `--random` | 只收集新鲜应用，并按随机顺序搜索 |
| `--skip-developer` | 跳过「同开发者的应用」收集 |
| `-v` / `--verbose` | 输出 DEBUG 日志 |
| `--disable-log-file` | 不写日志文件 |
| `--hdc-path <路径>` | 指定 HDC 可执行文件路径 |

提交搜索时，纯英文名称先按回车，其余情况点击「搜索」按钮。部分平板按回车会直接
跳转搜索结果页，此时搜索框的 key 从 `__SearchField__search_box*` 变成
`__SearchField__searchFrameInput*`，浮层里的「搜索」按钮随之消失；这种情况下按钮
缺失属于正常现象，程序以搜索结果页是否出现作为唯一判据，不再把按钮当作必需条件。

### 同开发者的应用

搜索成功后，程序会点击搜索结果里的第一个应用进入其详情页，向下滚动找到
「同开发者的应用」区块，点区块最右侧的「更多」进入同名列表页，划到底收集该开发者
的全部应用名。收集到的名称会并入待搜索名称，并在下一轮刷新遍历后继续搜索：

```text
搜索结果页 -> 第一个应用详情页 -> 下滑找到「同开发者的应用」-> 点「更多」
           -> 开发者应用列表页划到底 -> 原路返回搜索结果页
```

部分应用（例如开发者只上架了一个应用）的详情页没有这个区块，此时跳过当前应用；
偶发失败只记录警告并恢复搜索结果页，不会让本次搜索失败。使用 `--skip-developer`
可以完全关闭这一步。

## Rust UI 分类遍历

`hilog` 子命令实现了 Python 命令
`uv run .\main.py hilog --no-submit` 的纯 UI 部分。它会在每台在线设备上启动
AppGallery，遍历「应用」和「游戏」的分类页面，并将每个应用列表下滑至稳定。
默认不抓取 hilog，也不提交应用：

```powershell
cargo run --release -- hilog
```

可用参数包括 `--skip-categories <名称>...`、`--loop <次数>`、
`--loop-wait 5m`、`--ping 15` 和 `--keep-open-on-error`。`--submit`
为未来的 hilog 抓取和应用投稿流程预留；当前版本会明确拒绝该参数，避免悄悄执行
不完整的投稿流程。

偶发情况（等待分类内容或应用列表超时、当前页面没有应用卡片）只记录 warning 并跳过
当前分类，不会让整台设备失败，这与 Python `hilog --no-submit` 的容错行为一致。
只有结构性失败（找不到分类列表、滚动次数超过上限）仍会报错。
