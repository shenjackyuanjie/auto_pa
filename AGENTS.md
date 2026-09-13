# AGENTS.md

仓库级协作约定；动代码前先看这里。

## 代码格式化

统一用 nightly 的 rustfmt，稳定版不参与排版：

```powershell
cargo +nightly fmt          # 写入
cargo +nightly fmt --check  # 提交前检查，没有输出才算干净
```

- 仓库里没有 `rustfmt.toml`，排版完全由 rustfmt 的默认配置决定。换行、缩进不要手工调整，
  也不要为了让某处换行保留下来而加 `#[rustfmt::skip]`。
- 格式化改动单独成一个 `style:` 提交，不要和逻辑改动混在同一个提交里。
