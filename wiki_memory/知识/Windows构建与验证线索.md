---
type: knowledge
status: active
updated: 2026-11-09
topic: windows-build-validation
sources:
  - "src-tauri/build.rs"
  - "src-tauri/windows.manifest"
  - "src-tauri/Cargo.toml"
  - "scripts/desktop-smoke.mjs"
  - "scripts/test-queue.ps1"
  - "docs/validation.md"
---

# Windows 构建与验证线索

## Windows 清单

当前 `build.rs` 使用 Tauri `new_without_app_manifest()`，保留图标/版本资源，再通过链接器为所有 Windows 最终目标统一嵌入 `windows.manifest`。历史上缺 Common-Controls v6 会令测试/示例启动失败，重复 MANIFEST1 则令主程序链接失败；不能只给主 EXE 补清单。清单同时设置 asInvoker、DPI 和长路径。

该取舍由 Agent 在构建修复时采用，已用于现有本机通过的示例、Rust 测试和 NSIS。来源：`build.rs`、`windows.manifest`、`docs/validation.md`。修改清单方案时按实际受影响目标构建验证，不只看 cargo check。

## 测试入口和边界

`scripts/test-queue.ps1` 编译确定性假引擎并显式运行默认 ignored 的 worker 生命周期测试；普通 Rust 单测通过不代表该项已执行。假引擎不替代真实媒体测试。

`scripts/desktop-smoke.mjs` 生成原创测试图/音轨，经本地 HTTP、真实 yt-dlp、实际 Tauri 两窗口和 IPC 回归。它需要 debug EXE、FFmpeg、显式 yt-dlp 路径；会强制终止自己创建的应用进程树模拟异常退出，运行前按 README 关闭其他实例。隔离数据在被忽略的 `artifacts/`。调试端口仅 debug 且显式测试数据目录时启用，release 不包含入口。

公开媒体联调用 `src-tauri/examples/engine_smoke.rs`，报告数字只证明当时来源与环境。站点格式、网络、EJS 和外部引擎变化后，需要针对新任务核实，不能把旧报告当作现时通过。

## 证据与未核实项

2026-11-09 核对源码和历史记录；本轮仅工程记忆/文档维护，未重跑业务测试。Rust 配置最低 1.97；实际本机环境和完整结果见 `docs/validation.md`。远程 Actions、真实高 DPI 和安装卸载需要独立证据。
