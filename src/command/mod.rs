//! `auto-pa` 的子命令实现。
//!
//! 每个子命令一个模块，入口都叫 `run`：模块自己负责参数落地（构造 HDC 配置、发现设备、
//! 调度设备任务），真正与界面交互的流程放在 `crate::search` 与 `crate::hilog`。
//!
//! 两个子命令的设备调度策略一致：用 `HmDriver::discover_devices` 发现设备后只保留
//! `DeviceStatus::Online` 的设备，再用 `tokio::task::JoinSet` 并行执行；没有在线设备
//! 直接报错，其余失败等所有设备跑完后汇总成一条错误返回。

pub mod hilog;
pub mod search;
