# 开发与调试指南

面向修改 `auto-pa-rs` 的开发者：常用命令、日志开关、真机调试手法、控件树分析方法、
代码阅读顺序与常见坑。命令与结论均来自本仓库源码（`src/**`）与仓库内既有调试脚本
（`.cache/dev/ui.ps1`、`.cache/dev/scroll_to_end.ps1`、`py/src/hdc.py`）；未确认的内容
见文末「待确认」。

## 1. 常用命令

```powershell
cargo build                     # 开发构建（二进制 target/debug/auto-pa.exe）
cargo build --release           # 发布构建
cargo run -- search --fresh     # 从头开始跑 search
cargo run --release -- search --random --skip-developer
cargo run --release -- search -v
cargo run --release -- hilog --skip-categories 工具 游戏 --loop 2 --loop-wait 10m --ping 15
cargo clippy --all-targets      # 静态检查
cargo test                      # 单元测试
```

- `cargo run` 与子命令参数之间必须有 `--`，否则 clap 会把参数当成 cargo 的参数。
- `--hdc-path <路径>` 可指定 hdc 可执行文件；缺省使用 `HdcConfig::default()`。
- `--skip-categories` 使用 `num_args = 1..`，会一直吞掉后续值，因此要放在其它选项之后
  （或后面紧跟另一个以 `--` 开头的选项）。
- 单测分布在 `src/hilog/traversal.rs`、`src/search/state.rs`、`src/search/execution.rs`、
  `src/search/developer.rs`、`src/appgallery.rs`、`src/command/hilog.rs`，覆盖时长解析、
  进度文件格式、key 前缀匹配、页码签名、分类按钮过滤等纯逻辑，不依赖设备。

## 2. 日志与调试开关

| 开关 | 效果 |
| --- | --- |
| （默认） | 终端 + 文件均为 `INFO` |
| `-v` / `--verbose` | 级别切到 `DEBUG`（分类按钮发现、滚动稳定、回车提交、恢复尝试等日志） |
| `--disable-log-file` | 不创建日志文件，只输出到终端 |
| `--hdc-path <路径>` | 指定 hdc |
| `--keep-open-on-error`（仅 hilog） | 设备执行失败时保留 AppGallery 现场，便于观察出错时的 UI |

日志文件位置（`src/logging.rs`：`tracing_appender::rolling::daily("logs", file_name)`）：

```text
logs/search-rust.log.YYYY-MM-DD
logs/hilog-rust.log.YYYY-MM-DD
```

按天滚动，跨天运行会生成新文件；终端输出带 ANSI，文件不带颜色且不打印 target。
排查时：

```powershell
Get-ChildItem logs
Get-Content .\logs\search-rust.log.2026-09-12 -Tail 80
Get-Content .\logs\search-rust.log.2026-09-12 -Wait    # 实时跟踪
Select-String -Path .\logs\search-rust.log.* -Pattern "搜索失败|超时|同开发者"
```

`.gitignore` 已忽略 `*.log`、`logs/search-rust.log.*`、`logs/hilog-rust.log.*`。

## 3. 真机调试手法（hdc）

以下命令与参数来自仓库内既有脚本 `.cache/dev/ui.ps1`（封装 dump/click/input/key/swipe
等动作）与 `py/src/hdc.py`。

### 3.1 设备与布局树

```powershell
hdc list targets -v                                   # 列出设备（py/src/hdc.py refresh_targets）
hdc shell uitest dumpLayout -p /data/local/tmp/home.json
hdc file recv /data/local/tmp/home.json .\tmp\home.json
```

`py/src/hdc.py` 里的另一种取法是不落盘，直接从 stdout 拿路径：

```powershell
hdc shell "export T=$(uitest dumpLayout | cut -d ':' -f2-); cat $T; rm $T"
```

### 3.2 模拟交互

```powershell
hdc shell uitest uiInput click 1560 400                 # 点击坐标
hdc shell uitest uiInput inputText 1560 240 Facebook    # 在某坐标输入文本
hdc shell uitest uiInput keyEvent 2                     # 2 = 返回，1 = Home（.cache/dev/ui.ps1）
hdc shell uitest uiInput swipe 1560 1500 1560 600 400   # 向上滑一屏（参数：x1 y1 x2 y2 时长ms）
hdc shell uitest uiInput swipe 1560 600 1560 1500 400   # 向下滑一屏
```

### 3.3 AppGallery 生命周期

```powershell
hdc shell aa force-stop com.huawei.hmsapp.appgallery
hdc shell aa start -a MainAbility -b com.huawei.hmsapp.appgallery
```

包名与 Ability 与源码常量一致（`src/appgallery.rs`）。

### 3.4 截图

```powershell
hdc shell snapshot_display -f /data/local/tmp/shot.jpeg
hdc file recv /data/local/tmp/shot.jpeg .\tmp\shot.jpeg
```

本仓库现有脚本中**没有**使用该命令（见「待确认」），需要时先自行确认当前 hdc 版本的
参数名；`.cache/dev/` 中只留有历史截图文件。

### 3.5 既有封装脚本

`.cache/dev/ui.ps1` 把常见动作做成 `动词:参数` 形式，可当参考实现：

```powershell
.\.cache\dev\ui.ps1 dump:home click:1560,400 input:1560,240,Facebook key:2 up back start shell:"pm list packages"
```

`.cache/dev/scroll_to_end.ps1` 演示了「反复上滑 + dumpLayout + 统计 `app_name` 文本，
连续 2 轮无新增即到底」的验证手法，可用来独立复核 `collect_app_list` 的滚动逻辑。

## 4. 如何分析控件树

### 4.1 提取关键字段

布局 JSON 的属性节点形如 `{"attributes":{...}}`，属性包含 `type`、`key`、`text`、
`clickable`、`bounds`。`bounds` 是 `[x1,y1][x2,y2]` 字符串（`py/src/utils.py::parse_bounds`），
中心点即 `((x1+x2)/2, (y1+y2)/2)`，也是本项目点击时使用的坐标。

`.cache/dev/ui.ps1` 的做法可直接复用：用正则 `\{"attributes":\{([^{}]*)\}` 抓属性块，只保留
`key` 非空、`text` 非空或 `clickable == "true"` 的节点，按「bottom 再 left」排序，输出为
`type|key|text|clickable|bounds` 的文本文件。这样每次 dump 都能直接和上一次对比位置变化。

### 4.2 定位控件的优先级

1. **按 key 精确匹配**：最稳，例如 `SearchInputCard.Button.searchFrameBack`、`AppDetailPage`、
   `__NavdestinationField__Text__MainTitle__`、`app_name`。
2. **按 key 前缀匹配**：key 后缀会变时使用，例如搜索框 `__SearchField__search_box`
   （单测里出现的实例是 `__SearchField__search_box2`）。
3. **按 text 匹配**：`分类`、`应用`、`游戏`、`新鲜应用`、`同开发者的应用` 等。
4. **按属性组合推导**：文本节点本身不可点击时，向上找 `clickable == "true"` 的祖先节点
   （`py/src/utils.py` 注释说明 HarmonyOS 的 clickable 容器常常比标签高好几层；本项目在
   `click_local` 中用 `find_click_target` 取这类点击目标）。
5. **纯几何推导**：完全没有 key/text 的按钮，用相对位置描述。`src/search/developer.rs`
   定位「同开发者的应用」右侧「更多」入口就是例子：候选必须 `clickable == "true"`、
   `bounds.left >= 标题.right`、`bounds.top <= 标题中心行 y <= bounds.bottom`，再取其中最靠左者。

### 4.3 判断页面身份

- 「搜索结果页」：只要树里存在 `SearchInputCard.Button.searchFrameBack`
  （`has_key`）。这是 `search_once` 里判定搜索成功的**唯一判据**。
- 「应用详情页」：存在 key `AppDetailPage`。
- 「开发者应用列表页」：存在 `__NavdestinationField__Text__MainTitle__` 且其 text 等于
  `同开发者的应用`——详情页里同名的区块只是普通文本，没有这个标题 key，两者必须区分。
- 「分类页就绪」：`category_buttons(tree)` 非空。
- 「应用列表就绪」：`app_snapshot(tree)` 非空，且取的是 `app_name` 数量最多的那个 List。

## 5. 代码阅读顺序建议

1. `src/main.rs`：CLI 形状、日志初始化、子命令分发。
2. `src/command/search.rs`：参数语义、设备发现与并发调度。
3. `src/search/flow.rs`：`SearchFlow` 字段、主流程骨架、公共点击/滚动/等待辅助函数、
   全部时间常量。
4. `src/search/state.rs`：进度文件格式与序列号清洗（含路径 `.random` 规则）。
5. `src/search/collection.rs`：分类遍历与名称收集（含 `--random` 分支）。
6. `src/search/execution.rs`：搜索输入、提交方式、结果页判定、重试。
7. `src/search/developer.rs`：详情页区块滚动、无 key 按钮定位、原路返回与恢复。
8. `src/appgallery.rs`：控件树解析规则（分类按钮、应用卡片、文本黑名单）。
9. `src/command/hilog.rs` + `src/hilog/traversal.rs`：另一条命令线的遍历与容错策略。
10. `src/logging.rs`：日志落地细节。

## 6. 常见坑与排查清单

| 现象 | 原因 | 处理/依据 |
| --- | --- | --- |
| 纯英文查询提交后搜索框与「搜索」按钮的 key 从 `…search_box*` 变成 `…searchFrameInput1`，浮层按钮消失 | 平板布局下回车即提交并跳转结果页（`README.md` 亦有说明） | 源码只做 800ms 探测，按钮缺失按「已提交」处理，成功与否只看结果页 key 是否出现（`src/search/execution.rs`） |
| 搜索框 key 后缀数字每次不同 | 输入框实例编号会变 | 必须用前缀 `__SearchField__search_box` 匹配，禁止全等比较 |
| 点击「搜索」按钮超时 | 非英文查询下浮层未展开或页面尚未就绪（源码未区分具体原因；英文查询路径只探测 800ms 并容忍缺失） | 先 `ensure_search_home` 重置到应用页主页，再重试；单名称最多 3 次 |
| 详情页找不到「同开发者的应用」 | 部分应用详情页本身没有该区块 | 属于正常分支：`reveal_developer_section` 返回 false，收集数记 0，原路返回结果页，不是错误 |
| 详情页滚动 14 次仍没有该区块 | 达到 `DETAIL_SCROLL_MAX` | 记 warning 并按「没有该区块」处理 |
| 「更多」入口点不到 | 该入口没有 key 也没有 text | 只能按 clickable + 与标题同一行 + 位于标题右侧的几何规则定位；若规则失效需要重新 dump 详情页核对 bounds |
| 偶发「等待控件超时」但整体能跑完 | 页面切换/渲染抖动 | 靠重试自愈：搜索结果页 15s、结果列表 6s、详情页 15s；失败后 `home_ready = false` 重建主页 |
| hilog 报 warning「等待分类内容超时，按空内容继续」/「等待应用列表超时，按空列表继续」 | 偶发空帧 | 设计如此，只跳过当前分类；只有「未找到分类按钮」「滚动超过 100 次」才整台设备失败 |
| 中英文混排名称（如 `Facebook 微信`）或纯数字名称 | `is_english_query` 要求「含 ASCII 字母」且「整个字符串都是 ASCII」 | 这类查询走点击「搜索」按钮，而不是回车；调试时别只看名字里有没有英文 |
| 分类按钮识别多了/少了 | `is_category_tab_or_action` 黑名单为 `精选`、`分类`、`排行榜`、`重磅更新`、`安装`、`打开`、`更新`；且同名按钮只取一次 | 新增页面文案时应同步维护黑名单 |
| `--random` 之后进度看起来「丢了」 | `--random` 使用 `.cache/search/<序列号>.random.json`，与普通模式是两个文件 | 想从头来用 `--fresh`；注意 `--fresh` 不会删除旧文件，而是立刻覆盖写默认状态 |
| 进度文件报「版本不兼容」 | `version != 1` | 只能 `--fresh` 重来，或手工把 JSON 的 `version` 改成 1（字段顺序无关） |
| 进度文件保存失败 | Windows 不能直接覆盖重命名 | 实现是先写 `.json.tmp`，再删除原文件、`rename`；排查时留意是否有残留 tmp |
| hilog 报「--submit 的 hilog 抓取和应用投稿流程尚未实现」 | 该流程尚未实现 | 属于有意拒绝，不是 bug；`--username` 也必须与 `--submit` 同用 |
| 多设备日志分不清 | 设备标签是 `device-{index}`，index 是本次发现顺序 | 启动时的「发现设备」行里带 serial，与 index 对照 |
| 布局 dump 文件没进版本库 | `.gitignore` 忽略 `*.json`、`*.log`、`.cache` | 需要长期保留的样本要改后缀或另存 |

## 7. 待确认

- `hdc shell snapshot_display -f ...` 截图方式未在本仓库脚本或源码中出现，参数名需按当前
  hdc 版本自行验证（本仓库 dump 布局用的是 `uitest dumpLayout`）。
- `hm_driver_rs` 内部实际下发的 hdc 命令（包括等待 UI 的轮询间隔与超时语义）不在本仓库，
  只能从 `wait_for_ui` / `wait_for_ui_tree` 的调用点推断。
- 设备侧 uitest/Hypium agent 的版本与 `On.key` 支持情况只能从源码注释得知「部分 agent 未
  实现」，具体版本范围未知。
- 平板分辨率与「平板/PC 布局」的判定没有写在 Rust 源码里；仓库内只有 `py/` 通过
  `SP_daemon -deviceinfo` 的 `activeMode` 动态获取主屏尺寸。
- `.cache/dev/` 下的调试脚本与样本没有任何构建产物引用，是否长期保留由维护者决定。
