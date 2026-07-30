<div align="center">

# Auto_pa

</div>

## Rust UI search

The AppGallery UI search flow is implemented by `hm_driver_rs`:

```powershell
cargo run --release --bin auto-pa-search -- --fresh
```

The existing Python implementation is left unchanged and remains available:

```powershell
python search.py --fresh
```

Rust logs default to `INFO` and are written to both the terminal and
`logs/search-rust.log.YYYY-MM-DD`. Use `-v` for `DEBUG` output or
`--disable-log-file` to disable the file output.

After the current pending names are searched, the program performs one more
category traversal and searches only newly discovered names. Progress is kept
per device under `.cache/search/`.
