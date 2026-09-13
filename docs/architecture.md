# 架构说明

本文档描述 `auto-pa-rs`（二进制 `auto-pa`）的模块划分、数据流、进度文件、控件定位与依赖。
结论均来自仓库内源码（`src/**`、`Cargo.toml`、`Cargo.lock`、`README.md`、`.gitignore`），
无法确认的内容集中在文末「待确认」。

## 1. 工具定位

本工具是原 Python 版 AppGallery 自动化工具的 Rust 重写：Python 版本（根目录 `main.py`、
`search.py`、`auto_pa.py` 与 `py/`）已删除，需要查阅时用 `git show <commit>:<路径>` 从历史
里取。Rust 版本通过 `hm_driver_rs`
（内部依赖 hdc 与 uitest agent）操作华为平板上的 AppGallery。
包名 `auto-pa-rs`，二进制名 `auto-pa`（由 `Cargo.toml` 里的 `[[bin]] name = "auto-pa"`
显式声明，产物是 `target/{debug,release}/auto-pa.exe`；不写这一段的话产物会跟着包名叫
`auto-pa-rs.exe`），`edition = "2024"`，`publish = false`，描述为
`AppGallery UI search automation`；入口只有 `src/main.rs`，用 clap 定义两个子命令
（`#[command(name = "auto-pa", about = "AppGallery 自动化工具")]`）。

## 2. 子命令总览

| 子命令 | 作用 | 主要参数 |
| --- | --- | --- |
| `search` | 遍历「应用」「游戏」分类收集应用名 → 逐个搜索 → 从搜索结果第一个应用进入详情页收集「同开发者的应用」→ 刷新分类后再搜新增名称 | `--fresh`、`--random`、`--deep`、`--skip-developer`、`-v/--verbose`、`--disable-log-file`、`--hdc-path <路径>` |
| `hilog` | 遍历分类与应用列表；默认不抓 hilog、不投稿 | `-v/--verbose`、`--disable-log-file`、`--hdc-path <路径>`、`--skip-categories <分类>...`、`--loop <次数>`（默认 1）、`--loop-wait <时长>`（默认 `5m`）、`--ping <数值>`（默认 15）、`--keep-open-on-error`、`--submit`、`--username <名称>` |

`hilog` 的参数约束（`src/command/hilog.rs`）：`--loop` 必须大于 0；`--username` 只能与 `--submit`
同用；`--submit` 直接报错「hilog 抓取和应用投稿流程尚未实现」。`--loop-wait` 支持 `5m`、
`1h30m`、`00h00m05s` 或纯秒数（`parse_duration`）。

两个子命令都按 `--hdc-path` 构造 `HdcConfig`（缺省 `HdcConfig::default()`），都用 `HmDriver::discover_devices`
发现设备且只保留 `DeviceStatus::Online`，无在线设备时报「未发现在线 HarmonyOS 设备」；每台设备在
`JoinSet` 中并发执行（标签 `device-{index}`），失败时汇总为「{N} 台设备执行失败：…」。

## 3. 目录结构

```text
src/main.rs / src/lib.rs   入口与共用模块声明（clap CLI、模块导出）
src/logging.rs             tracing 初始化：终端 + 按日滚动文件，INFO/DEBUG
src/appgallery.rs          包名/Ability 常量、控件树解析（分类按钮、应用卡片）
src/command/               search.rs / hilog.rs：参数、时长解析、设备发现与调度
src/search/flow.rs         SearchFlow：启停 AppGallery、分类遍历骨架、通用点击/滚动/等待
src/search/collection.rs   分类遍历与应用名收集（含 --random 的「新鲜应用」分支）
src/search/execution.rs    逐个名称搜索、提交方式判定、结果页判定与收集
src/search/developer.rs    打开结果页第一个应用、收集「同开发者的应用」并原路返回
src/search/state.rs        进度文件读写（SearchState / SearchStateStore）、序列号清洗
src/hilog/traversal.rs     分类/子分类/应用列表遍历，容错式等待；mod.rs 负责导出
scripts/                   真机 UI 调试脚本（dumpLayout、点击、滑动、返回）；.cache/、logs/ 为运行期产物
```

## 4. 整体数据流

### 4.1 search 主流程（`search::flow::SearchFlow::run`）

```text
发现设备(仅 Online) -> 每设备一个任务
  -> SearchStateStore::for_device(serial, random_mode) -> load(--fresh)
  -> collection_complete == false 时：启动 AppGallery -> 遍历分类收集名称 -> 写进度并置位
  -> 搜索所有 pending_names（失败只记 warning，不中断）
  -> 刷新遍历一次分类（新增名称并入 app_names）-> 再搜索 pending_names -> 关闭应用与 HmDriver
```

- 页签顺序：先 `start_appgallery()`，再处理 `["应用"]`；非 `--random` 时追加 `"游戏"`。每个页签
  走 `click_app_or_game` → `click_categories_tab` → `pull_categories`；后者最多
  `MAX_CATEGORY_SCROLLS = 100` 轮，连续 2 轮无新分类按钮即到底，超过上限报
  「分类列表超过 [100] 次仍未到底」。
- 进入分类后按深度分三条路径：默认直接等待应用列表并读 3 屏（`0..=2`），每屏向上滑一次；
  `--random` 时先在 5 秒内等待文本 `新鲜应用`、点进去再读 3 屏；`--deep` 则等分类页内容，
  出现 `SUBCATEGORY_NAMES`（新鲜应用/新鲜游戏/时下畅销应用/时下畅销游戏）里的入口就逐个进入、
  每个列表由 `collect_app_list` 滑到底，否则把分类首页自己滑到底。三条路径最后都用
  `back_to_categories`（默认 1 次、`--random` 2 次）回到分类列表，再保存该分类新增名称。
- 爬取阶段的等待全部降级：等子分类/应用列表超时只记 warning 并按当前 UI 继续，空页面结束
  当前层级；只有找不到分类按钮、超过滚动上限这类结构性异常才报错（与 `hilog` 一致）。
- 名称去重与累积由 `SearchState::add_apps` 完成（空名不入库，返回新增条数）；首轮失败的名称不会
  被 `mark_searched`，因此下一轮会重试。

### 4.2 单个名称的搜索（`src/search/execution.rs`）

```text
ensure_search_home(): 启动 AppGallery -> 点「应用」页签 -> 等 __SearchField__search_box* (12s)
search_once(name):
  点搜索输入框(前缀, 12s) -> 200ms -> input_text -> 300ms -> 英文查询按 Enter -> 100ms
  点「搜索」按钮(前缀)：英文查询超时 800ms，其余 5s；英文查询下按钮缺失视为已提交
  等结果页 key SearchInputCard.Button.searchFrameBack (15s) -> 取结果页布局(6s)
  developer_scan 且有布局时 collect_developer_apps；再 collect_app_list 统计结果数
  点结果页返回按钮(8s) -> 等搜索输入框回到主页(15s)
```

`search_pending` 对每个名称最多尝试 `MAX_SEARCH_ATTEMPTS = 3` 次，每次失败把 `home_ready`
置 false 并重建搜索主页；三次都失败则本轮报「有 N 个应用搜索失败，下次运行会重试：…」。
`--random` 时 pending 列表用 `SliceRandom::shuffle` 打乱。

### 4.3 同开发者应用收集（`src/search/developer.rs`）

```text
结果页按 (top, left) 取第一个应用卡片 -> 点击 -> 等 key AppDetailPage (15s)
读详情页头部的开发者名：文本「开发者」标签 + 同列的 detail_bottom_name 值（不用滚动）
    本轮已收集过该开发者（SearchFlow::collected_developers）-> 直接退回结果页，返回 0
逐屏下滑最多 DETAIL_SCROLL_MAX = 14 次，直到出现文本「同开发者的应用」
    前后两屏「全部非空文本排序去重拼接」签名相同 -> 判定到底，视为没有该区块，返回 0
读区块内的应用卡片：标题下方的 app_name，范围止于包含标题的最小 ListItem 的底边
    数量 < DEVELOPER_PREVIEW_LIMIT = 9 -> 区块没填满，就是全部应用，直接入库并返回一次
    填满或读不到卡片 -> 点区块标题同一行、位于标题右侧的最左可点击节点（该入口没有 key、没有 text）
        等列表页：key __NavdestinationField__Text__MainTitle__ 且 text == 同开发者的应用 (12s)
        collect_app_list(allow_empty=false) 划到底 -> add_apps -> 写进度 -> 返回
    收完记下这个开发者；同一个开发者的其它应用本轮不再走上面两步
再逐级后退直到结果页返回按钮出现 -> 等结果页 (12s)
```

详情页区块一次最多渲染三行三列（实测 5 个应用的开发者只展示 5 个，73 个应用的开发者展示
9 个满行），所以「没填满」可以安全地当作完整列表，省掉一次进列表页再退回的往返。

开发者名的去重只省时间、不承担正确性：读不到名字（没有标签、值不在同列）或收集失败时不记名单，
下一个同开发者的应用会照旧完整走一遍；名单在每轮 `collect_all_categories` 开头清空，因此刷新
遍历之后重新收集。读取规则与实机几何见 `detail_developer_name` 的文档与单测。

异常时 `back_to_result_page()`：最多 3 次尝试，树里存在 `SearchInputCard.Button.searchFrameBack`
即认为已在结果页，否则 `go_back`；正常收尾与中断恢复共用它。

### 4.4 hilog 主流程（`src/hilog/traversal.rs`）

`UiTraversal::run` 启动 AppGallery 后固定遍历 `["应用", "游戏"]`，没有 `--random` / `--deep` 分支；
分类页内存在 `SUBCATEGORY_NAMES` 子分类入口时逐个进入并划到底，否则直接遍历当前应用列表，
`--skip-categories` 命中的分类只记日志并跳过。`wait_for_category_content` / `wait_for_app_list`
超时只记 warning 并退化为当前 UI（空内容跳过该分类），只有「找不到分类按钮」「滚动超过上限」这类
结构性失败才报错；`--keep-open-on-error` 在失败时保留 AppGallery 现场（仅关闭 HmDriver）。

`search --deep` 复用的是同一套深度与容错：进入分类后的子分类识别、滑动到底与等待降级都与本节
描述一致，区别只在 `search` 会把读到的名称并入进度。

## 5. 进度文件

- 路径：`.cache/search/<清洗后的序列号>[.random].json`（`SearchStateStore::for_device`；`--random` 追加
  `.random`，两种模式各自独立）。序列号清洗 `sanitize_serial` 保留 ASCII 字母数字与 `_ . -`、其余替换为
  `_`，去掉首尾 `.` 与 `_`，结果为空时用 `device`。

| 字段 | 类型 | 语义（`SearchState`，`STATE_VERSION = 1`） |
| --- | --- | --- |
| `version` | number | 状态格式版本，必须为 `1`；否则报「搜索进度文件版本不兼容，请使用 --fresh 重新开始」 |
| `collection_complete` | bool | 初始分类遍历是否完成；true 时跳过初始遍历，只做刷新遍历 |
| `app_names` | string[] | 已收集的全部应用名（去重、无空串），按收集顺序 |
| `searched_names` | string[] | 已成功搜索的名称；`pending_names()` = `app_names` 中不在其中的项 |

- 写入时机：`--fresh` 初始化后、每个分类收集完成后、`collection_complete` 置位后、每个名称搜索
  成功后、同开发者应用并入后，均整体重写。写入方式：先写 `<路径>.json.tmp`，再删除原文件并
  `rename` 覆盖（注释说明 Windows 不能直接覆盖重命名），内容为 `serde_json::to_string_pretty`；
  读取时 `--fresh` 或文件不存在即返回默认状态，否则解析 JSON 并校验版本与空名。

## 6. 日志

`logging::init(verbose, disable_log_file, file_name)` 由 `main.rs` 调用，`file_name` 分别为
`search-rust.log` 与 `hilog-rust.log`：

| 指标 | 行为 |
| --- | --- |
| 级别 / 关闭文件 | 默认 `INFO`，`-v/--verbose` 为 `DEBUG`；`--disable-log-file` 只初始化终端输出，不创建文件（返回 `None` guard） |
| 终端 / 文件 | 终端带 ANSI 颜色；文件走 `tracing_appender::rolling::daily("logs", file_name)`，形如 `logs/search-rust.log.YYYY-MM-DD` 与 `logs/hilog-rust.log.YYYY-MM-DD`，无 ANSI、非阻塞写入 |
| 生命周期 | 非阻塞写入的 `WorkerGuard` 由 `main` 持有（`_log_guard`），随进程结束落盘 |

`.gitignore` 显式忽略 `logs/search-rust.log.*` 与 `logs/hilog-rust.log.*`。

## 7. 关键控件 key / 文本索引

除标注「前缀」外，源码一律要求 key 或 text **精确相等**。

| key / text / 属性 | 匹配方式 | 用途 | 出现页面 | 源码 |
| --- | --- | --- | --- | --- |
| `app_name` | key | 应用卡片名称节点，`text` 即应用名，`bounds` 用于点击 | 分类/搜索结果/开发者应用列表页 | `appgallery.rs` |
| `BadgeImage.sys.symbol.bag_fill`、`BadgeImage.sys.symbol.game_fill`、`Paf_Lantern_Button_Index_1`、`Paf_Lantern_Text_1`、`Paf_Lantern_Normal_Image_1`、`Paf_Lantern_Select_Image_1` | key（命中任一） | 「应用」「游戏」页签图标，「分类」入口；均与对应 text（`应用`、`游戏`、`分类`）并列判断 | AppGallery 首页 | `search/flow.rs`、`hilog/traversal.rs` |
| `__SearchField__search_box`、`__SearchField__Button__search_box` | key **前缀** | 搜索输入框、搜索按钮；后缀数字会变，必须前缀匹配 | 应用页搜索浮层 | `search/execution.rs` |
| `SearchInputCard.Button.searchFrameBack` | key | 结果页返回按钮，同时是「结果页已打开」的唯一判据 | 搜索结果页 | `search/execution.rs`、`search/developer.rs` |
| `AppDetailPage`、`__NavdestinationField__Text__MainTitle__` + text `同开发者的应用` | key（后者需 key + text） | 确认应用详情页已打开；开发者列表页靠主标题 key 与同名 text 同时命中来区分详情页里的同名区块 | 应用详情页 / 开发者应用列表页 | `search/developer.rs` |
| text `开发者` + `detail_bottom_name` | text（标签）+ key（值） | 详情页头部「标签 + 值」四列（安装量/年龄/分类/开发者）共用值 key，按「与标签同列（中心 x 相差 ≤ 60px）」取开发者名，用于同一开发者的去重 | 应用详情页头部 | `search/developer.rs` |
| 「更多」入口 | **无 key、无 text** | 按 `clickable == "true"`、垂直覆盖标题行中心、`left >= 标题右边界`，取 `left` 最大者（最靠右） | 详情页区块标题行右侧 | `search/developer.rs` |
| `新鲜应用`；`新鲜应用`、`新鲜游戏`、`时下畅销应用`、`时下畅销游戏` | text | `--random` 模式下分类内入口；`--deep` 与 hilog 的子分类入口（`SUBCATEGORY_NAMES`） | 分类页 | `search/collection.rs`、`appgallery.rs` |
| `精选`、`分类`、`排行榜`、`重磅更新`、`安装`、`打开`、`更新` | text | 分类按钮黑名单（`is_category_tab_or_action`） | 分类页 | `appgallery.rs` |
| `type == "List"`、`type == "Button"`、`clickable == "true"` | 属性 | 列表容器（分类取「按钮最多」、应用取「应用最多」的 List）、分类按钮（需有非空 text 子树）、无 key 按钮兜底 | 分类页、列表页、详情页 | `appgallery.rs`、`search/developer.rs` |

`appgallery.rs` 单测记录：实机新版 AppGallery 子分类入口层级为 `clickable Column -> Row -> Text`，`Row`/`Text` 自身不是 `Button`，因此子分类按 text 找 clickable 祖先而非要求 `Button` 类型（`--deep` 与 hilog 共用这份识别）。

## 8. 关键常量与超时

| 常量 | 值 | 位置 |
| --- | --- | --- |
| `MAX_CATEGORY_SCROLLS` / `MAX_SCROLLS` | 均 100（分类列表 / 应用列表滚动上限） | `search/flow.rs`、`search/collection.rs`、`hilog/traversal.rs` |
| `APP_STOP_SETTLE` / `APP_START_SETTLE` / `PAGE_CLICK_SETTLE` / `CATEGORY_CLICK_SETTLE` / `CATEGORY_SCROLL_SETTLE` / `CATEGORY_CONTENT_TIMEOUT` | 1s / 3s / 750ms / 1s / 850ms / 5s | `search/flow.rs`、`hilog/traversal.rs` |
| `BACK_SETTLE` / `DETAIL_SCROLL_SETTLE` / `SCROLL_BATCH_SIZE` / `SCROLL_WAIT` | 1500ms / 700ms / 2 / 100ms | `search/flow.rs`、`search/collection.rs` |
| 搜索相关：`MAX_SEARCH_ATTEMPTS` / 输入等待 / `SEARCH_BUTTON_TIMEOUT` / 回车后探测 / `RESULT_LIST_TIMEOUT` | 3 / 200ms+300ms+100ms / 5s / 800ms / 6s | `search/execution.rs` |
| `DETAIL_SCROLL_MAX` / `DEVELOPER_PREVIEW_LIMIT` / `DETAIL_PAGE_TIMEOUT` / `DEVELOPER_LIST_TIMEOUT` / `RESULT_PAGE_TIMEOUT` / `RECOVER_ATTEMPTS` | 14 / 9 / 15s / 12s / 12s / 3 | `search/developer.rs` |
| hilog 分类点击等待 | `1.0 + ping * 0.05` 秒（`--ping 15` 约 1.75 秒） | `hilog/traversal.rs` |

滚动统一为 `swipe_direction(SwipeDirection::Up, SwipeArea::FullScreen, 0.7, 2_000)`。

## 9. 依赖与外部工具

| 依赖 | 版本 / 特性 | 用途 |
| --- | --- | --- |
| `hm_driver_rs` | `1.0.1`，`default-features = false`，`features = ["embedded-agents"]` | 设备发现、连接、UI 树、点击/输入/按键/滑动、应用启停 |
| `clap` / `tokio` | `4`（`derive/std/help/usage/error-context`）/ `1`（`macros/rt-multi-thread/time`） | CLI、异步运行时与 `JoinSet` |
| `anyhow` / `serde` / `serde_json` / `tracing` / `tracing-subscriber` / `tracing-appender` / `rand` | `1` / `1` / `1` / `0.1` / `0.3` / `0.2` / `0.10` | 错误上下文、进度文件序列化、日志与 `--random` 打乱顺序 |

外部工具：hdc（路径可由 `--hdc-path` 指定）与设备侧 uitest/Hypium agent（由 `embedded-agents`
特性驱动）。源码注释指出部分 Hypium agent 未实现 `On.key`，因此本项目统一用本地布局树按 key/bounds
匹配后再点击。AppGallery 常量：包名 `com.huawei.hmsapp.appgallery`，Ability `MainAbility`。

## 10. 构建与运行

```powershell
cargo build / cargo build --release  # 开发构建（target/debug/auto-pa.exe）/ 发布构建
cargo run -- search --fresh
cargo run --release -- search --random --skip-developer
cargo run --release -- hilog --skip-categories 工具 游戏 --loop 2 --loop-wait 10m
```

`cargo run` 与子命令之间需要 `--`；`README.md` 推荐的调用形式是 `cargo run --release -- <子命令> [参数]`。

## 待确认

- `hm_driver_rs` 内部下发的 hdc / uitest 命令与 agent 生命周期不在本仓库，只能从调用点推断（`wait_for_ui`、
  `wait_for_ui_tree`、`ui_tree`、`click`、`input_text`、`press_key_code`、`swipe_direction`、`go_back`、
  `start_app`、`stop_app`、`wait_for_app`、`discover_devices`）。
- 平板分辨率与「平板/PC 布局」判断未在 Rust 源码中硬编码：Rust 侧只按控件树的 key/bounds
  匹配，不读取 `SP_daemon -deviceinfo` 的 `activeMode` 主屏尺寸。
- `hilog` 的 hilog 抓取与投稿流程（`--submit`）尚未实现，架构上无对应模块。被删除的
  Python 实现里曾有完整的投稿链路（`py/src/hmgallery.py` 的后端 HTTP 客户端、`auto_pa.py`
  的分享面板操作、`py/core/hilog.py` 的 hilog 解析与投稿），后续要用 `git show` 取回。
- `scripts/` 下的真机调试脚本不被源码或 `Cargo.toml` 引用，仅供调试参考。
